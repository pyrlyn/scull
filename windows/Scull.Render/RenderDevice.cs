using Windows.Win32;
using Windows.Win32.Graphics.Direct3D;
using Windows.Win32.Graphics.Direct3D11;
using Windows.Win32.Graphics.Dxgi.Common;
using Windows.Win32.System.Com;

namespace Scull.Render;

/// <summary>
/// A Direct3D 11 device and its immediate context. WARP, the software
/// rasteriser every Windows ships, renders the same picture with no GPU: what
/// tests and a machine without a usable driver get.
/// </summary>
public sealed unsafe class RenderDevice : IDisposable
{
    private ID3D11Device* device;
    private ID3D11DeviceContext* context;

    public RenderDevice(bool warp = false)
    {
        // Feature level 10 is enough for instancing with SV_VertexID; asking
        // for no more keeps old GPUs and every WARP version in.
        ReadOnlySpan<D3D_FEATURE_LEVEL> levels =
        [
            D3D_FEATURE_LEVEL.D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL.D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL.D3D_FEATURE_LEVEL_10_0,
        ];
        ID3D11Device* d;
        ID3D11DeviceContext* c;
        // BGRA support: the swap chain and the atlas use BGRA, as DirectWrite's bitmaps do.
        PInvoke.D3D11CreateDevice(null, warp ? D3D_DRIVER_TYPE.D3D_DRIVER_TYPE_WARP : D3D_DRIVER_TYPE.D3D_DRIVER_TYPE_HARDWARE, default,
            D3D11_CREATE_DEVICE_FLAG.D3D11_CREATE_DEVICE_BGRA_SUPPORT, levels, PInvoke.D3D11_SDK_VERSION, &d, out _, &c).ThrowOnFailure();
        device = d;
        context = c;
        IsWarp = warp;
    }

    public bool IsWarp { get; }

    internal ID3D11Device* Device => device != null ? device : throw new ObjectDisposedException(nameof(RenderDevice));

    internal ID3D11DeviceContext* Context => context != null ? context : throw new ObjectDisposedException(nameof(RenderDevice));

    public void Dispose()
    {
        Com.Release(ref context);
        Com.Release(ref device);
    }

    internal ID3D11Texture2D* CreateTexture(int width, int height, D3D11_BIND_FLAG bind, D3D11_USAGE usage = D3D11_USAGE.D3D11_USAGE_DEFAULT,
        D3D11_CPU_ACCESS_FLAG cpu = 0)
    {
        var desc = new D3D11_TEXTURE2D_DESC
        {
            Width = (uint)width,
            Height = (uint)height,
            MipLevels = 1,
            ArraySize = 1,
            Format = DXGI_FORMAT.DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc = new DXGI_SAMPLE_DESC { Count = 1 },
            Usage = usage,
            BindFlags = bind,
            CPUAccessFlags = cpu,
        };
        ID3D11Texture2D* texture;
        Device->CreateTexture2D(&desc, null, &texture);
        return texture;
    }
}

internal static unsafe class Com
{
    /// <summary>Releases a COM pointer once and clears it, so a second Dispose does nothing.</summary>
    public static void Release<T>(ref T* pointer)
        where T : unmanaged
    {
        T* p = pointer;
        pointer = null;
        if (p != null)
        {
            ((IUnknown*)p)->Release();
        }
    }
}

/// <summary>
/// Where a frame is drawn: a BGRA render target view of a known size. The
/// swap chain's back buffer is one (T17.3); <see cref="OffscreenTarget"/> is
/// the other.
/// </summary>
public abstract unsafe class RenderTarget : IDisposable
{
    public int Width { get; protected init; }

    public int Height { get; protected init; }

    internal abstract ID3D11RenderTargetView* View { get; }

    public abstract void Dispose();
}

/// <summary>A texture to render into and read back: tests, snapshots and the benchmark.</summary>
public sealed unsafe class OffscreenTarget : RenderTarget
{
    private readonly RenderDevice device;
    private ID3D11Texture2D* texture;
    private ID3D11Texture2D* staging;
    private ID3D11RenderTargetView* view;

    public OffscreenTarget(RenderDevice device, int width, int height)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(width, 1);
        ArgumentOutOfRangeException.ThrowIfLessThan(height, 1);
        (this.device, Width, Height) = (device, width, height);
        texture = device.CreateTexture(width, height, D3D11_BIND_FLAG.D3D11_BIND_RENDER_TARGET);
        staging = device.CreateTexture(width, height, 0, D3D11_USAGE.D3D11_USAGE_STAGING, D3D11_CPU_ACCESS_FLAG.D3D11_CPU_ACCESS_READ);
        ID3D11RenderTargetView* v;
        device.Device->CreateRenderTargetView((ID3D11Resource*)texture, null, &v);
        view = v;
    }

    internal override ID3D11RenderTargetView* View => view;

    /// <summary>The picture as BGRA rows, top first; waits for the GPU.</summary>
    public byte[] ReadPixels()
    {
        ID3D11DeviceContext* context = device.Context;
        context->CopyResource((ID3D11Resource*)staging, (ID3D11Resource*)texture);
        D3D11_MAPPED_SUBRESOURCE mapped;
        context->Map((ID3D11Resource*)staging, 0, D3D11_MAP.D3D11_MAP_READ, 0, &mapped);
        var pixels = new byte[Width * Height * 4];
        for (int y = 0; y < Height; y++)
        {
            new ReadOnlySpan<byte>((byte*)mapped.pData + (nint)y * mapped.RowPitch, Width * 4).CopyTo(pixels.AsSpan(y * Width * 4));
        }
        context->Unmap((ID3D11Resource*)staging, 0);
        return pixels;
    }

    public override void Dispose()
    {
        Com.Release(ref view);
        Com.Release(ref staging);
        Com.Release(ref texture);
    }
}
