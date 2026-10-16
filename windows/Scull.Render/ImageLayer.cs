using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Scull.Core;
using Windows.Win32.Graphics.Direct3D11;
using Windows.Win32.Graphics.Dxgi.Common;
using Windows.Win32.System.Com;

namespace Scull.Render;

/// <summary>Where an image is drawn, in pixels from the grid's top-left, and the texels it samples.</summary>
internal readonly record struct ImageQuad(float X, float Y, float W, float H, ushort U, ushort V, ushort SpanU, ushort SpanV);

/// <summary>A placement's cell box, pixel offset and source rectangle, as the frame gives them.</summary>
internal readonly record struct ImageBox(int Row, uint Col, uint Cols, uint Rows, uint OffsetX, uint OffsetY,
    uint SourceX, uint SourceY, uint SourceWidth, uint SourceHeight)
{
    public static ImageBox Of(in Placement p) =>
        new(p.Row, p.Col, p.Cols, p.Rows, p.OffsetX, p.OffsetY, p.SourceX, p.SourceY, p.SourceWidth, p.SourceHeight);

    public bool IsEmpty => Cols == 0 || Rows == 0 || SourceWidth == 0 || SourceHeight == 0;

    /// <summary>The quad that draws this box from an image of <paramref name="width"/> x <paramref name="height"/> pixels; false when nothing shows.</summary>
    public bool TryLayout(int width, int height, CellMetrics m, out ImageQuad quad)
    {
        quad = default;
        if (IsEmpty || width <= 0 || height <= 0 || width > ushort.MaxValue || height > ushort.MaxValue)
        {
            return false;
        }
        // The source rectangle stays inside the pixels and within what a
        // quad's 16-bit texel fields carry, as ImageLayer.swift clamps it.
        uint srcX = Math.Min(SourceX, (uint)width - 1), srcY = Math.Min(SourceY, (uint)height - 1);
        uint srcW = Math.Min(SourceWidth, (uint)width - srcX), srcH = Math.Min(SourceHeight, (uint)height - srcY);
        quad = new ImageQuad((float)((double)Col * m.Width + OffsetX), (float)((double)Row * m.Height + OffsetY),
            (float)((double)Cols * m.Width), (float)((double)Rows * m.Height), (ushort)srcX, (ushort)srcY, (ushort)srcW, (ushort)srcH);
        return true;
    }
}

/// <summary>
/// The frame's image placements as textured quads, as ImageLayer.swift does:
/// the core hands out decoded RGBA pixels, each image and generation becomes
/// an immutable texture once and is reused while a placement shows it.
/// </summary>
[SupportedOSPlatform("windows8.1")]
internal sealed unsafe class ImageLayer : IDisposable
{
    private readonly record struct Texture(nint View, int Width, int Height);

    // The largest texture feature level 11 guarantees; a bigger image is not drawn.
    private const int MaxSide = 16384;

    private readonly RenderDevice device;
    private readonly ImageCache<Texture> textures = new(static t => ((IUnknown*)t.View)->Release());
    private nint[] views = new nint[4];
    private Renderer.Quad[] quads = new Renderer.Quad[4];
    private bool[] below = new bool[4];

    public ImageLayer(RenderDevice device) => this.device = device;

    /// <summary>Placements laid out by the last <see cref="Place"/>, lowest z first.</summary>
    public int Count { get; private set; }

    /// <summary>Textures held, for tests of their release.</summary>
    public int Textures => textures.Count;

    public Renderer.Quad[] Quads => quads;

    public ID3D11ShaderResourceView* View(int i) => (ID3D11ShaderResourceView*)views[i];

    /// <summary>A negative z draws under the text.</summary>
    public bool Below(int i) => below[i];

    /// <summary>Lays out <paramref name="view"/>'s placements and drops the textures of images no longer placed.</summary>
    public void Place(FrameView view, CellMetrics m)
    {
        Count = 0;
        foreach (Placement p in view.Placements)
        {
            ImageBox box = ImageBox.Of(p);
            if (p.Image == 0 || box.IsEmpty)
            {
                continue;
            }
            if (!textures.TryGet(p.Image, p.Generation, out Texture texture))
            {
                if (Upload(p) is not Texture made)
                {
                    continue;
                }
                texture = made;
                textures.Add(p.Image, p.Generation, texture);
            }
            if (!box.TryLayout(texture.Width, texture.Height, m, out ImageQuad q))
            {
                continue;
            }
            if (Count == quads.Length)
            {
                Array.Resize(ref quads, Count * 2);
                Array.Resize(ref views, Count * 2);
                Array.Resize(ref below, Count * 2);
            }
            quads[Count] = new Renderer.Quad
            {
                X = q.X, Y = q.Y, W = q.W, H = q.H, U = q.U, V = q.V, SpanU = q.SpanU, SpanV = q.SpanV,
                Color = 0xFFFF_FFFF, Kind = Renderer.Image,
            };
            (views[Count], below[Count]) = (texture.View, p.Z < 0);
            Count++;
        }
        textures.EndFrame();
    }

    private Texture? Upload(in Placement p)
    {
        ReadOnlySpan<byte> pixels = p.Pixels(out int width, out int height, out int stride);
        if (pixels.IsEmpty || width > MaxSide || height > MaxSide)
        {
            return null;
        }
        var desc = new D3D11_TEXTURE2D_DESC
        {
            Width = (uint)width,
            Height = (uint)height,
            MipLevels = 1,
            ArraySize = 1,
            Format = DXGI_FORMAT.DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc = new DXGI_SAMPLE_DESC { Count = 1 },
            Usage = D3D11_USAGE.D3D11_USAGE_IMMUTABLE,
            BindFlags = D3D11_BIND_FLAG.D3D11_BIND_SHADER_RESOURCE,
        };
        ID3D11Texture2D* texture = null;
        ID3D11ShaderResourceView* view = null;
        fixed (byte* data = pixels)
        {
            var initial = new D3D11_SUBRESOURCE_DATA { pSysMem = data, SysMemPitch = (uint)stride };
            try
            {
                device.Device->CreateTexture2D(&desc, &initial, &texture);
                device.Device->CreateShaderResourceView((ID3D11Resource*)texture, null, &view);
            }
            catch (Exception e) when (e is COMException or ArgumentException or OutOfMemoryException)
            {
                // A device that refuses one image draws the rest of the frame.
                Com.Release(ref view);
                return null;
            }
            finally
            {
                Com.Release(ref texture);
            }
        }
        return new Texture((nint)view, width, height);
    }

    public void Dispose() => textures.Clear();
}
