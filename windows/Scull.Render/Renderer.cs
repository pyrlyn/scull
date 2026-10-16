using System.Numerics;
using System.Runtime.InteropServices;
using Scull.Core;
using System.Runtime.Versioning;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.Direct3D;
using Windows.Win32.Graphics.Direct3D11;
using Windows.Win32.Graphics.Dxgi.Common;

namespace Scull.Render;

/// <summary>
/// Draws a frame with Direct3D 11, the design of MetalRenderer.swift: every
/// cell background, glyph, decoration and the cursor is one instanced quad.
/// Rows are built on the CPU only when the frame's damage says they changed,
/// kept in slots of one instance buffer and reached through a row-to-slot
/// table, so a scroll only reorders the table; a rebuilt slot is uploaded
/// alone. Nothing is allocated per frame once the glyphs on screen are in the
/// atlas. One thread at a time, the one that owns the device's context.
/// </summary>
[SupportedOSPlatform("windows8.1")]
public sealed unsafe class Renderer : IDisposable
{
    // Same layout as `Instance` in Shaders.cs.
    [StructLayout(LayoutKind.Sequential)]
    internal struct Quad
    {
        public float X, Y, W, H;
        public ushort U, V;
        public uint Color;
        public uint Kind;
        public ushort SpanU, SpanV;
    }

    internal const uint Image = 3;
    private const uint Solid = 0, Coverage = 1, ColorGlyph = 2;
    private const int CursorQuads = 24;

    private readonly RenderDevice device;
    private FontFaces fonts;
    private AtlasPacker packer = new();
    private readonly SpriteCanvas sprites = new();
    private RunShaper shaper;
    private readonly ImageLayer images;
    private Palette palette;
    private readonly Quad[] cursor = new Quad[CursorQuads];
    private ID3D11VertexShader* vertexShader;
    private ID3D11PixelShader* pixelShader;
    private ID3D11InputLayout* layout;
    private ID3D11BlendState* blend;
    private ID3D11RasterizerState* raster;
    private ID3D11Buffer* uniforms;
    private ID3D11Buffer* overlay;
    private ID3D11Buffer* pictures;
    private ID3D11SamplerState* sampler;
    private int picturesCapacity;
    private ID3D11Buffer* instances;
    private ID3D11Texture2D* atlas;
    private ID3D11ShaderResourceView* atlasView;
    private int atlasSize;
    private int cols, rows, capBg, capFg, stride;
    private Quad[] quads = [];
    private int[] bgCount = [], fgCount = [], slotOfRow = [], sources = [], before = [], next = [], free = [], build = [];
    private bool[] taken = [];
    private HashSet<ushort>[] shelvesOf = [];
    private ulong frame;
    private bool needsRebuild = true;
    private int areaWidth, areaHeight;

    public Renderer(RenderDevice device, string fontFamily, float fontSizePixels, Palette? palette = null)
    {
        this.device = device;
        this.palette = palette ?? new Palette();
        fonts = new FontFaces(fontFamily, fontSizePixels);
        try
        {
            shaper = new RunShaper(fonts);
            images = new ImageLayer(device);
            CreatePipeline();
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    public CellMetrics Metrics => fonts.Metrics;

    /// <summary>The colours rows are drawn in; a new palette rebuilds every row.</summary>
    public Palette Palette
    {
        get => palette;
        set
        {
            palette = value ?? throw new ArgumentNullException(nameof(value));
            needsRebuild = true;
        }
    }

    /// <summary>
    /// Draws with another font from the next frame: new faces, shaped runs and
    /// atlas, the pipeline kept. A missing family throws
    /// <see cref="ArgumentException"/> and leaves the current font in place.
    /// The cell size may change, so the caller sends the terminal its new grid.
    /// </summary>
    public void SetFont(string fontFamily, float fontSizePixels)
    {
        var next = new FontFaces(fontFamily, fontSizePixels);
        shaper.Dispose();
        fonts.Dispose();
        fonts = next;
        shaper = new RunShaper(fonts);
        // Glyphs and sprites were drawn at the old size; rows name the old shelves.
        packer = new AtlasPacker();
        foreach (HashSet<ushort> shelves in shelvesOf)
        {
            shelves.Clear();
        }
        Com.Release(ref atlasView);
        Com.Release(ref atlas);
        atlasSize = 0;
        needsRebuild = true;
    }

    /// <summary>A focused view draws a block cursor, an unfocused one a hollow box.</summary>
    public bool Focused { get; set; } = true;

    /// <summary>Distinct glyphs in the atlas, blanks included.</summary>
    public int AtlasEntries => packer.Count;

    public int AtlasGeneration => packer.Generation;

    /// <summary>Image textures held, for tests of their release.</summary>
    internal int ImageTextures => images.Textures;

    private ID3D11DeviceContext* Context => device.Context;

    private void CreatePipeline()
    {
        ID3DBlob* vs = Shaders.Compile("vs", "vs_4_0");
        ID3DBlob* ps = null;
        try
        {
            ps = Shaders.Compile("ps", "ps_4_0");
            ID3D11Device* d = device.Device;
            fixed (ID3D11VertexShader** v = &vertexShader)
            {
                d->CreateVertexShader(vs->GetBufferPointer(), vs->GetBufferSize(), null, v);
            }
            fixed (ID3D11PixelShader** p = &pixelShader)
            {
                d->CreatePixelShader(ps->GetBufferPointer(), ps->GetBufferSize(), null, p);
            }
            fixed (byte* rect = "RECT"u8, uv = "UV"u8, color = "COLOR"u8, kind = "KIND"u8, span = "SPAN"u8)
            fixed (ID3D11InputLayout** l = &layout)
            {
                D3D11_INPUT_ELEMENT_DESC* elements = stackalloc D3D11_INPUT_ELEMENT_DESC[]
                {
                    Element(rect, DXGI_FORMAT.DXGI_FORMAT_R32G32B32A32_FLOAT, 0),
                    Element(uv, DXGI_FORMAT.DXGI_FORMAT_R16G16_UINT, 16),
                    Element(color, DXGI_FORMAT.DXGI_FORMAT_R8G8B8A8_UNORM, 20),
                    Element(kind, DXGI_FORMAT.DXGI_FORMAT_R32_UINT, 24),
                    Element(span, DXGI_FORMAT.DXGI_FORMAT_R16G16_UINT, 28),
                };
                d->CreateInputLayout(elements, 5, vs->GetBufferPointer(), vs->GetBufferSize(), l);
            }
        }
        finally
        {
            Com.Release(ref ps);
            Com.Release(ref vs);
        }
        var blendDesc = new D3D11_BLEND_DESC();
        blendDesc.RenderTarget._0 = new D3D11_RENDER_TARGET_BLEND_DESC
        {
            BlendEnable = true,
            SrcBlend = D3D11_BLEND.D3D11_BLEND_ONE,
            DestBlend = D3D11_BLEND.D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp = D3D11_BLEND_OP.D3D11_BLEND_OP_ADD,
            SrcBlendAlpha = D3D11_BLEND.D3D11_BLEND_ONE,
            DestBlendAlpha = D3D11_BLEND.D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha = D3D11_BLEND_OP.D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask = (byte)D3D11_COLOR_WRITE_ENABLE.D3D11_COLOR_WRITE_ENABLE_ALL,
        };
        // No culling: a quad's winding is whatever its corners give.
        var rasterDesc = new D3D11_RASTERIZER_DESC { FillMode = D3D11_FILL_MODE.D3D11_FILL_SOLID, CullMode = D3D11_CULL_MODE.D3D11_CULL_NONE, DepthClipEnable = true };
        var samplerDesc = new D3D11_SAMPLER_DESC
        {
            Filter = D3D11_FILTER.D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU = D3D11_TEXTURE_ADDRESS_MODE.D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV = D3D11_TEXTURE_ADDRESS_MODE.D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW = D3D11_TEXTURE_ADDRESS_MODE.D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD = float.MaxValue,
        };
        fixed (ID3D11BlendState** b = &blend)
        fixed (ID3D11RasterizerState** r = &raster)
        fixed (ID3D11SamplerState** s = &sampler)
        {
            device.Device->CreateBlendState(&blendDesc, b);
            device.Device->CreateRasterizerState(&rasterDesc, r);
            device.Device->CreateSamplerState(&samplerDesc, s);
        }
        uniforms = Buffer(16, D3D11_BIND_FLAG.D3D11_BIND_CONSTANT_BUFFER);
        overlay = Buffer(CursorQuads * sizeof(Quad), D3D11_BIND_FLAG.D3D11_BIND_VERTEX_BUFFER);
    }

    private static D3D11_INPUT_ELEMENT_DESC Element(byte* name, DXGI_FORMAT format, uint offset) => new()
    {
        SemanticName = new PCSTR(name),
        Format = format,
        AlignedByteOffset = offset,
        InputSlotClass = D3D11_INPUT_CLASSIFICATION.D3D11_INPUT_PER_INSTANCE_DATA,
        InstanceDataStepRate = 1,
    };

    private ID3D11Buffer* Buffer(int bytes, D3D11_BIND_FLAG bind)
    {
        var desc = new D3D11_BUFFER_DESC { ByteWidth = (uint)bytes, Usage = D3D11_USAGE.D3D11_USAGE_DEFAULT, BindFlags = bind };
        ID3D11Buffer* buffer;
        device.Device->CreateBuffer(&desc, null, &buffer);
        return buffer;
    }

    /// <summary>
    /// Folds the damage of one <see cref="Terminal.Update"/> into what the next
    /// <see cref="Render"/> repaints. Call it after every update, drawn or not:
    /// the core reports damage per update, and several may come between draws.
    /// </summary>
    public void Absorb(FrameView view)
    {
        if (!view.Updated)
        {
            return;
        }
        int n = view.Rows;
        if (view.Full || n != sources.Length || view.Cols != cols)
        {
            needsRebuild = true;
            return;
        }
        // Every scroll reads the picture as it was before this update.
        sources.CopyTo(before, 0);
        foreach (Scroll scroll in view.Scrolls)
        {
            for (int row = scroll.Start; row < Math.Min((int)scroll.End, n); row++)
            {
                int source = scroll.From + row - scroll.Start;
                sources[row] = source < n ? before[source] : -1;
            }
        }
        foreach (ushort row in view.Dirty)
        {
            if (row < n)
            {
                sources[row] = -1;
            }
        }
    }

    /// <summary>Draws <paramref name="view"/> into <paramref name="target"/>, the grid from its top-left pixel.</summary>
    public void Render(FrameView view, RenderTarget target) => Render(view, target, null);

    /// <summary>
    /// Draws <paramref name="view"/> into <paramref name="region"/> of <paramref name="target"/>, the grid from the
    /// region's top-left pixel: a pane of a window whose panes share one swap chain. The region is filled with the
    /// background and nothing outside it is touched; null is the whole target, cleared.
    /// </summary>
    public void Render(FrameView view, RenderTarget target, Region? region)
    {
        if (view.Cols != cols || view.Rows != rows)
        {
            Allocate(view.Cols, view.Rows);
        }
        // The ABI sizes the cells to the grid; anything else is not drawn.
        if (view.Cells.Length < rows * cols)
        {
            return;
        }
        frame++;
        int dirty = needsRebuild ? rows : Remap();
        if (needsRebuild)
        {
            for (int row = 0; row < rows; row++)
            {
                (slotOfRow[row], build[row]) = (row, row);
            }
        }
        needsRebuild = false;
        // Shelves that reused rows draw from must survive this frame's evictions.
        foreach (HashSet<ushort> shelves in shelvesOf)
        {
            foreach (ushort shelf in shelves)
            {
                packer.Touch(shelf, frame);
            }
        }
        int generation = packer.Generation;
        for (int i = 0; i < dirty; i++)
        {
            Build(build[i], view);
        }
        // A grown atlas dropped every entry, so rows built before the growth
        // point at texels that are gone.
        for (int attempt = 0; attempt < 4 && packer.Generation != generation; attempt++)
        {
            generation = packer.Generation;
            for (int row = 0; row < rows; row++)
            {
                Build(row, view);
            }
        }
        for (int row = 0; row < rows; row++)
        {
            sources[row] = row;
        }
        Draw(view, target, region);
    }

    private void Allocate(int newCols, int newRows)
    {
        Com.Release(ref instances);
        (cols, rows) = (newCols, newRows);
        // A row has at most one background per cell; the cap on the rest
        // keeps decorations and wide glyphs within the slot.
        (capBg, capFg) = (cols, cols * 4 + 16);
        stride = capBg + capFg;
        quads = new Quad[checked(rows * stride)];
        (bgCount, fgCount, slotOfRow, sources) = (new int[rows], new int[rows], new int[rows], new int[rows]);
        (before, next, free, build, taken) = (new int[rows], new int[rows], new int[rows], new int[rows], new bool[rows]);
        shelvesOf = new HashSet<ushort>[rows];
        for (int row = 0; row < rows; row++)
        {
            shelvesOf[row] = [];
            (slotOfRow[row], sources[row]) = (row, row);
        }
        instances = Buffer(checked(Math.Max(1, quads.Length) * sizeof(Quad)), D3D11_BIND_FLAG.D3D11_BIND_VERTEX_BUFFER);
        needsRebuild = true;
    }

    /// <summary>Gives reused rows their old slots and the rest the free ones; answers how many rows of <c>build</c> need building.</summary>
    private int Remap()
    {
        Array.Clear(taken);
        for (int row = 0; row < rows; row++)
        {
            int source = sources[row];
            next[row] = -1;
            if (source >= 0 && !taken[slotOfRow[source]])
            {
                next[row] = slotOfRow[source];
                taken[next[row]] = true;
            }
        }
        int freeCount = 0;
        for (int slot = 0; slot < rows; slot++)
        {
            if (!taken[slot])
            {
                free[freeCount++] = slot;
            }
        }
        int dirty = 0;
        for (int row = 0; row < rows; row++)
        {
            if (next[row] < 0)
            {
                next[row] = free[--freeCount];
                build[dirty++] = row;
            }
        }
        (slotOfRow, next) = (next, slotOfRow);
        return dirty;
    }

    private static uint Rgba(uint rgb, uint alpha = 255) => (rgb >> 16 & 0xFF) | (rgb & 0xFF00) | (rgb & 0xFF) << 16 | alpha << 24;

    private static Quad Rect(int x, int y, int w, int h, uint color) => new() { X = x, Y = y, W = w, H = h, Color = color, Kind = Solid };

    private void Build(int row, FrameView view)
    {
        int slot = slotOfRow[row];
        int bgBase = slot * stride, fgBase = bgBase + capBg, nb = 0, nf = 0;
        HashSet<ushort> shelves = shelvesOf[slot];
        shelves.Clear();
        CellMetrics m = fonts.Metrics;
        ReadOnlySpan<Cell> cells = view.Cells.Slice(row * cols, cols);
        if (row < view.Lines.Length)
        {
            Row line = view.Lines[row];
            foreach (Run run in line.Runs)
            {
                Style style = view.StyleAt(run.Style);
                StyleAttributes attrs = style.Attributes;
                if ((attrs & StyleAttributes.Hidden) != 0)
                {
                    continue;
                }
                uint alpha = (attrs & StyleAttributes.Dim) != 0 ? 128u : 255u;
                uint color = Rgba(palette.Colors(style).Fg, alpha);
                int face = ((attrs & StyleAttributes.Bold) != 0 ? 1 : 0) | ((attrs & StyleAttributes.Italic) != 0 ? 2 : 0);
                foreach (ShapedGlyph g in shaper.Shape(line.TextOf(run), face, run.Width))
                {
                    int col = run.Col + g.Column;
                    if (col >= cols || nf >= capFg)
                    {
                        break;
                    }
                    // The current match's background is bright in any theme, so its text has its own colour.
                    uint tint = cells[col].IsCurrentMatch ? Rgba(palette.CurrentMatchText, alpha) : color;
                    // Pinned to its cluster's column, so a fallback's own
                    // advances never push the rest of the run off the grid.
                    Glyph(Entry(g.Key), col * m.Width + (int)MathF.Round(g.Dx), tint, alpha, (int)MathF.Round(g.Dy));
                }
            }
        }
        int start = 0;
        uint current = palette.Background;
        for (int col = 0; col <= cols; col++)
        {
            Cell cell = col < cols ? cells[col] : default;
            Style style = view.StyleAt(cell.Style);
            (uint fg, uint bg) = palette.Colors(style);
            bg = palette.Highlight(cell, bg);
            // Neighbouring cells of one colour share a quad; the default
            // background is the clear colour.
            if (col == cols || bg != current)
            {
                if (current != palette.Background && col > start)
                {
                    quads[bgBase + nb++] = Rect(start * m.Width, 0, (col - start) * m.Width, m.Height, Rgba(current));
                }
                (start, current) = (col, bg);
            }
            StyleAttributes attrs = style.Attributes;
            if (col == cols || cell.Width == 0 || (attrs & StyleAttributes.Hidden) != 0)
            {
                continue;
            }
            uint alpha = (attrs & StyleAttributes.Dim) != 0 ? 128u : 255u;
            uint color = Rgba(fg, alpha);
            int x = col * m.Width, w = cell.Width * m.Width;
            // One sprite per column, so a curly line keeps one period per cell.
            if (style.Underline != Underline.None && nf < capFg)
            {
                AtlasEntry e = Entry(Sprites.UnderlineKey(style.Underline));
                uint tint = Rgba(palette.Rgb(style.UnderlineColor, fg), alpha);
                for (int c = 0; c < cell.Width; c++)
                {
                    Glyph(e, x + c * m.Width, tint, alpha);
                }
            }
            if ((attrs & StyleAttributes.Strike) != 0 && nf < capFg)
            {
                quads[fgBase + nf++] = Rect(x, m.Strike, w, m.Thickness, color);
            }
            if ((attrs & StyleAttributes.Overline) != 0 && nf < capFg)
            {
                quads[fgBase + nf++] = Rect(x, 0, w, m.Thickness, color);
            }
        }
        (bgCount[slot], fgCount[slot]) = (nb, nf);
        // One upload per rebuilt row: its backgrounds' region and its glyphs.
        var box = new D3D11_BOX { left = (uint)(bgBase * sizeof(Quad)), right = (uint)((fgBase + nf) * sizeof(Quad)), bottom = 1, back = 1 };
        fixed (Quad* q = &quads[bgBase])
        {
            Context->UpdateSubresource((ID3D11Resource*)instances, 0, &box, q, 0, 0);
        }

        void Glyph(AtlasEntry e, int pen, uint color, uint alpha, int rise = 0)
        {
            if (e.IsEmpty || nf >= capFg)
            {
                return;
            }
            quads[fgBase + nf++] = new Quad
            {
                X = pen + e.Left, Y = m.Baseline - rise - e.Top, W = e.Width, H = e.Height, U = e.X, V = e.Y,
                Color = e.IsColor ? Rgba(0xFFFFFF, alpha) : color, Kind = e.IsColor ? ColorGlyph : Coverage,
            };
            shelves.Add(e.Shelf);
        }
    }

    private AtlasEntry Entry(GlyphKey key)
    {
        if (packer.TryGet(key, frame, out AtlasEntry entry))
        {
            return entry;
        }
        bool drawn = key.Face == Sprites.Face ? sprites.Draw(key, fonts.Metrics, out GlyphBitmap bitmap) : fonts.Rasterize(key, out bitmap);
        if (!drawn)
        {
            return packer.AddEmpty(key);
        }
        // Not remembered when it fits nowhere: a later eviction may make room.
        if (packer.Place(key, bitmap.Width, bitmap.Height, (short)bitmap.Left, (short)bitmap.Top, bitmap.IsColor, frame) is not AtlasEntry placed)
        {
            return default;
        }
        if (atlasSize != packer.Size)
        {
            // Growth dropped every entry, so the old texels are not needed.
            Com.Release(ref atlasView);
            Com.Release(ref atlas);
            atlasSize = packer.Size;
            atlas = device.CreateTexture(atlasSize, atlasSize, D3D11_BIND_FLAG.D3D11_BIND_SHADER_RESOURCE);
            fixed (ID3D11ShaderResourceView** v = &atlasView)
            {
                device.Device->CreateShaderResourceView((ID3D11Resource*)atlas, null, v);
            }
        }
        var box = new D3D11_BOX { left = placed.X, top = placed.Y, right = (uint)(placed.X + placed.Width), bottom = (uint)(placed.Y + placed.Height), back = 1 };
        fixed (byte* pixels = bitmap.Pixels)
        {
            Context->UpdateSubresource((ID3D11Resource*)atlas, 0, &box, pixels, (uint)(bitmap.Width * 4), 0);
        }
        return placed;
    }

    private void Draw(FrameView view, RenderTarget target, Region? region)
    {
        ID3D11DeviceContext* c = Context;
        ID3D11RenderTargetView* rtv = target.View;
        uint bg = palette.Background;
        float* clear = stackalloc float[] { (bg >> 16 & 0xFF) / 255f, (bg >> 8 & 0xFF) / 255f, (bg & 0xFF) / 255f, 1 };
        Region area = region ?? new Region(0, 0, target.Width, target.Height);
        (areaWidth, areaHeight) = (area.Width, area.Height);
        if (region is null)
        {
            c->ClearRenderTargetView(rtv, clear);
        }
        c->OMSetRenderTargets(1, &rtv, null);
        // The viewport also clips, so a pane can never draw over its neighbour.
        var viewport = new D3D11_VIEWPORT { TopLeftX = area.X, TopLeftY = area.Y, Width = area.Width, Height = area.Height, MaxDepth = 1 };
        c->RSSetViewports(1, &viewport);
        c->RSSetState(raster);
        // A null blend factor: the span overload would demand four floats.
        c->OMSetBlendState(blend, (float*)null, 0xFFFF_FFFF);
        c->IASetInputLayout(layout);
        c->IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY.D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        c->VSSetShader(vertexShader, null, 0);
        c->PSSetShader(pixelShader, null, 0);
        ID3D11Buffer* u = uniforms;
        c->VSSetConstantBuffers(0, 1, &u);
        ID3D11ShaderResourceView* a = atlasView;
        c->PSSetShaderResources(0, 1, &a);
        ID3D11SamplerState* s = sampler;
        c->PSSetSamplers(0, 1, &s);
        images.Place(view, fonts.Metrics);
        UploadPictures();
        if (region is not null)
        {
            // A region cannot be cleared by ClearRenderTargetView, so one background quad does it.
            cursor[0] = Rect(0, 0, area.Width, area.Height, Rgba(palette.Background));
            var fill = new D3D11_BOX { right = (uint)sizeof(Quad), bottom = 1, back = 1 };
            fixed (Quad* q = cursor)
            {
                c->UpdateSubresource((ID3D11Resource*)overlay, 0, &fill, q, 0, 0);
            }
            Bind(overlay);
            Origin(0);
            c->DrawInstanced(4, 1, 0, 0);
        }
        (int under, int over) = Cursor(view);
        if (under + over > 0)
        {
            var box = new D3D11_BOX { right = (uint)((under + over) * sizeof(Quad)), bottom = 1, back = 1 };
            fixed (Quad* q = cursor)
            {
                c->UpdateSubresource((ID3D11Resource*)overlay, 0, &box, q, 0, 0);
            }
        }
        int height = fonts.Metrics.Height;
        for (int layer = 0; layer < 2; layer++)
        {
            Bind(instances);
            for (int row = 0; row < rows; row++)
            {
                int slot = slotOfRow[row];
                int count = layer == 0 ? bgCount[slot] : fgCount[slot];
                if (count > 0)
                {
                    Origin(row * height);
                    c->DrawInstanced(4, (uint)count, 0, (uint)(slot * stride + (layer == 0 ? 0 : capBg)));
                }
            }
            // Images with a negative z go under the text, the rest over it.
            // The cursor block goes over the backgrounds; the glyph under it
            // is drawn again on top in the background colour.
            for (int i = 0; i < images.Count; i++)
            {
                if (images.Below(i) == (layer == 0))
                {
                    Bind(pictures);
                    Origin(0);
                    ID3D11ShaderResourceView* picture = images.View(i);
                    c->PSSetShaderResources(1, 1, &picture);
                    c->DrawInstanced(4, 1, 0, (uint)i);
                }
            }
            int n = layer == 0 ? under : over;
            if (n > 0)
            {
                Bind(overlay);
                Origin(0);
                c->DrawInstanced(4, (uint)n, 0, layer == 0 ? 0u : (uint)under);
            }
        }
    }

    private void UploadPictures()
    {
        int count = images.Count;
        if (count == 0)
        {
            return;
        }
        if (count > picturesCapacity)
        {
            // Grown by doubling, so a steady set of images allocates nothing.
            Com.Release(ref pictures);
            picturesCapacity = Math.Max(4, (int)BitOperations.RoundUpToPowerOf2((uint)count));
            pictures = Buffer(picturesCapacity * sizeof(Quad), D3D11_BIND_FLAG.D3D11_BIND_VERTEX_BUFFER);
        }
        var box = new D3D11_BOX { right = (uint)(count * sizeof(Quad)), bottom = 1, back = 1 };
        fixed (Quad* q = images.Quads)
        {
            Context->UpdateSubresource((ID3D11Resource*)pictures, 0, &box, q, 0, 0);
        }
    }

    private void Bind(ID3D11Buffer* buffer)
    {
        uint quadStride = (uint)sizeof(Quad), offset = 0;
        Context->IASetVertexBuffers(0, 1, &buffer, &quadStride, &offset);
    }

    private void Origin(int y)
    {
        float* values = stackalloc float[] { areaWidth, areaHeight, 0, y };
        Context->UpdateSubresource((ID3D11Resource*)uniforms, 0, null, values, 0, 0);
    }

    /// <summary>Fills <c>cursor</c>: the quads under the text, then those over it.</summary>
    private (int Under, int Over) Cursor(FrameView view)
    {
        Core.Cursor at = view.Cursor;
        if (!at.Visible || at.Row >= rows || at.Col >= cols)
        {
            return (0, 0);
        }
        CellMetrics m = fonts.Metrics;
        int left = at.Col * m.Width, top = at.Row * m.Height;
        int width = Math.Max(1, (int)view.Cells[at.Row * cols + at.Col].Width) * m.Width;
        uint color = Rgba(palette.Cursor);
        if (!Focused)
        {
            cursor[0] = Rect(left, top, width, m.Thickness, color);
            cursor[1] = Rect(left, top + m.Height - m.Thickness, width, m.Thickness, color);
            cursor[2] = Rect(left, top, m.Thickness, m.Height, color);
            cursor[3] = Rect(left + width - m.Thickness, top, m.Thickness, m.Height, color);
            return (4, 0);
        }
        cursor[0] = Rect(left, top, width, m.Height, color);
        int over = 0, slot = slotOfRow[at.Row];
        for (int i = 0; i < fgCount[slot] && over < CursorQuads - 1; i++)
        {
            Quad q = quads[slot * stride + capBg + i];
            float centre = q.X + q.W / 2;
            if (q.Kind == Coverage && centre >= left && centre < left + width)
            {
                q.Y += top;
                q.Color = Rgba(palette.Background);
                cursor[1 + over++] = q;
            }
        }
        return (1, over);
    }

    public void Dispose()
    {
        Com.Release(ref atlasView);
        Com.Release(ref atlas);
        Com.Release(ref instances);
        images?.Dispose();
        Com.Release(ref pictures);
        Com.Release(ref sampler);
        Com.Release(ref overlay);
        Com.Release(ref uniforms);
        Com.Release(ref raster);
        Com.Release(ref blend);
        Com.Release(ref layout);
        Com.Release(ref pixelShader);
        Com.Release(ref vertexShader);
        shaper?.Dispose();
        fonts.Dispose();
    }
}
