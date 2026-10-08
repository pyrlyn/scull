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
    }

    private const uint Solid = 0, Coverage = 1, ColorGlyph = 2;
    private const int CursorQuads = 24;

    private readonly RenderDevice device;
    private readonly FontFaces fonts;
    private readonly AtlasPacker packer = new();
    private readonly Palette palette;
    private readonly Quad[] cursor = new Quad[CursorQuads];
    private ID3D11VertexShader* vertexShader;
    private ID3D11PixelShader* pixelShader;
    private ID3D11InputLayout* layout;
    private ID3D11BlendState* blend;
    private ID3D11RasterizerState* raster;
    private ID3D11Buffer* uniforms;
    private ID3D11Buffer* overlay;
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

    public Renderer(RenderDevice device, string fontFamily, float fontSizePixels, Palette? palette = null)
    {
        this.device = device;
        this.palette = palette ?? new Palette();
        fonts = new FontFaces(fontFamily, fontSizePixels);
        try
        {
            CreatePipeline();
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    public CellMetrics Metrics => fonts.Metrics;

    /// <summary>A focused view draws a block cursor, an unfocused one a hollow box.</summary>
    public bool Focused { get; set; } = true;

    /// <summary>Distinct glyphs in the atlas, blanks included.</summary>
    public int AtlasEntries => packer.Count;

    public int AtlasGeneration => packer.Generation;

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
            fixed (byte* rect = "RECT"u8, uv = "UV"u8, color = "COLOR"u8, kind = "KIND"u8)
            fixed (ID3D11InputLayout** l = &layout)
            {
                D3D11_INPUT_ELEMENT_DESC* elements = stackalloc D3D11_INPUT_ELEMENT_DESC[]
                {
                    Element(rect, DXGI_FORMAT.DXGI_FORMAT_R32G32B32A32_FLOAT, 0),
                    Element(uv, DXGI_FORMAT.DXGI_FORMAT_R16G16_UINT, 16),
                    Element(color, DXGI_FORMAT.DXGI_FORMAT_R8G8B8A8_UNORM, 20),
                    Element(kind, DXGI_FORMAT.DXGI_FORMAT_R32_UINT, 24),
                };
                d->CreateInputLayout(elements, 4, vs->GetBufferPointer(), vs->GetBufferSize(), l);
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
        fixed (ID3D11BlendState** b = &blend)
        fixed (ID3D11RasterizerState** r = &raster)
        {
            device.Device->CreateBlendState(&blendDesc, b);
            device.Device->CreateRasterizerState(&rasterDesc, r);
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
    public void Render(FrameView view, RenderTarget target)
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
        Draw(view, target);
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
        int start = 0;
        uint current = palette.Background;
        for (int col = 0; col <= cols; col++)
        {
            Cell cell = col < cols ? cells[col] : default;
            Style style = view.StyleAt(cell.Style);
            (uint fg, uint bg) = palette.Colors(style);
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
            if (cell.CodePoint > ' ' && nf < capFg)
            {
                AtlasEntry e = Entry(fonts.Lookup(cell.CodePoint, (attrs & StyleAttributes.Bold) != 0, (attrs & StyleAttributes.Italic) != 0));
                if (!e.IsEmpty)
                {
                    quads[fgBase + nf++] = new Quad
                    {
                        X = x + e.Left, Y = m.Baseline - e.Top, W = e.Width, H = e.Height, U = e.X, V = e.Y,
                        Color = e.IsColor ? Rgba(0xFFFFFF, alpha) : color, Kind = e.IsColor ? ColorGlyph : Coverage,
                    };
                    shelves.Add(e.Shelf);
                }
            }
            // Every underline kind is a straight line until sprites arrive (T17.5).
            if (style.Underline != Underline.None && nf < capFg)
            {
                quads[fgBase + nf++] = Rect(x, m.Underline, w, m.Thickness, Rgba(palette.Rgb(style.UnderlineColor, fg), alpha));
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
    }

    private AtlasEntry Entry(GlyphKey key)
    {
        if (packer.TryGet(key, frame, out AtlasEntry entry))
        {
            return entry;
        }
        if (!fonts.Rasterize(key, out GlyphBitmap bitmap))
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

    private void Draw(FrameView view, RenderTarget target)
    {
        ID3D11DeviceContext* c = Context;
        ID3D11RenderTargetView* rtv = target.View;
        uint bg = palette.Background;
        float* clear = stackalloc float[] { (bg >> 16 & 0xFF) / 255f, (bg >> 8 & 0xFF) / 255f, (bg & 0xFF) / 255f, 1 };
        c->ClearRenderTargetView(rtv, clear);
        c->OMSetRenderTargets(1, &rtv, null);
        var viewport = new D3D11_VIEWPORT { Width = target.Width, Height = target.Height, MaxDepth = 1 };
        c->RSSetViewports(1, &viewport);
        c->RSSetState(raster);
        c->OMSetBlendState(blend, null, 0xFFFF_FFFF);
        c->IASetInputLayout(layout);
        c->IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY.D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        c->VSSetShader(vertexShader, null, 0);
        c->PSSetShader(pixelShader, null, 0);
        ID3D11Buffer* u = uniforms;
        c->VSSetConstantBuffers(0, 1, &u);
        ID3D11ShaderResourceView* a = atlasView;
        c->PSSetShaderResources(0, 1, &a);
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
                    Origin(target, row * height);
                    c->DrawInstanced(4, (uint)count, 0, (uint)(slot * stride + (layer == 0 ? 0 : capBg)));
                }
            }
            // The cursor block goes over the backgrounds; the glyph under it
            // is drawn again on top in the background colour.
            int n = layer == 0 ? under : over;
            if (n > 0)
            {
                Bind(overlay);
                Origin(target, 0);
                c->DrawInstanced(4, (uint)n, 0, layer == 0 ? 0u : (uint)under);
            }
        }
    }

    private void Bind(ID3D11Buffer* buffer)
    {
        uint quadStride = (uint)sizeof(Quad), offset = 0;
        Context->IASetVertexBuffers(0, 1, &buffer, &quadStride, &offset);
    }

    private void Origin(RenderTarget target, int y)
    {
        float* values = stackalloc float[] { target.Width, target.Height, 0, y };
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
        Com.Release(ref overlay);
        Com.Release(ref uniforms);
        Com.Release(ref raster);
        Com.Release(ref blend);
        Com.Release(ref layout);
        Com.Release(ref pixelShader);
        Com.Release(ref vertexShader);
        fonts.Dispose();
    }
}
