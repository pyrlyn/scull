using System.Runtime.Versioning;
using Windows.Win32;
using Windows.Win32.Graphics.Direct3D11;
using Windows.Win32.Graphics.Dxgi;
using Windows.Win32.Graphics.Dxgi.Common;
using Windows.Win32.System.Com;

namespace Scull.Render;

/// <summary>
/// A swap chain for composition, the kind a WinUI <c>SwapChainPanel</c> shows:
/// one per window (AGENTS.md), panes being viewports of it. The host hands
/// <see cref="NativeSwapChain"/> to the panel once, then draws into this
/// target and calls <see cref="Present"/>.
/// </summary>
[SupportedOSPlatform("windows8.1")]
public sealed unsafe class SwapChainTarget : RenderTarget
{
    // Two buffers are the least flip presentation takes; more only adds latency.
    private const uint BufferCount = 2;

    private readonly RenderDevice device;
    private IDXGISwapChain2* swapChain;
    private ID3D11RenderTargetView* view;

    public SwapChainTarget(RenderDevice device, int width, int height)
    {
        this.device = device;
        (Width, Height) = (Math.Max(1, width), Math.Max(1, height));
        IDXGIDevice* dxgiDevice = null;
        IDXGIAdapter* adapter = null;
        IDXGIFactory2* factory = null;
        IDXGISwapChain1* created = null;
        try
        {
            dxgiDevice = Query<IDXGIDevice>(device.Device);
            dxgiDevice->GetAdapter(&adapter);
            Guid factoryId = IDXGIFactory2.IID_Guid;
            adapter->GetParent(&factoryId, (void**)&factory);
            // Composition takes only the flip model and stretching (the
            // CreateSwapChainForComposition reference); premultiplied alpha is
            // what it composes, and every pixel drawn here is opaque anyway.
            var desc = new DXGI_SWAP_CHAIN_DESC1
            {
                Width = (uint)Width,
                Height = (uint)Height,
                Format = DXGI_FORMAT.DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc = new DXGI_SAMPLE_DESC { Count = 1 },
                BufferUsage = DXGI_USAGE.DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount = BufferCount,
                Scaling = DXGI_SCALING.DXGI_SCALING_STRETCH,
                SwapEffect = DXGI_SWAP_EFFECT.DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode = DXGI_ALPHA_MODE.DXGI_ALPHA_MODE_PREMULTIPLIED,
            };
            factory->CreateSwapChainForComposition((IUnknown*)device.Device, &desc, null, &created);
            swapChain = Query<IDXGISwapChain2>(created);
            CreateView();
        }
        catch
        {
            Dispose();
            throw;
        }
        finally
        {
            Com.Release(ref created);
            Com.Release(ref factory);
            Com.Release(ref adapter);
            Com.Release(ref dxgiDevice);
        }
    }

    /// <summary>The <c>IDXGISwapChain</c> for <c>ISwapChainPanelNative.SetSwapChain</c>, which takes its own reference.</summary>
    public nint NativeSwapChain => (nint)swapChain;

    internal override ID3D11RenderTargetView* View => view;

    /// <summary>
    /// Resizes the buffers to <paramref name="width"/> x <paramref name="height"/>
    /// pixels, shown at <paramref name="scaleX"/> x <paramref name="scaleY"/> pixels
    /// per view unit. The panel scales its content by its composition scale, so
    /// the inverse here keeps one buffer pixel on one screen pixel.
    /// </summary>
    public void Resize(int width, int height, float scaleX, float scaleY)
    {
        (Width, Height) = (Math.Max(1, width), Math.Max(1, height));
        Com.Release(ref view);
        // ResizeBuffers fails while the context still binds the old buffer.
        device.Context->OMSetRenderTargets(0, null, null);
        swapChain->ResizeBuffers(BufferCount, (uint)Width, (uint)Height, DXGI_FORMAT.DXGI_FORMAT_B8G8R8A8_UNORM, 0);
        var inverse = new DXGI_MATRIX_3X2_F { _11 = 1 / scaleX, _22 = 1 / scaleY };
        swapChain->SetMatrixTransform(&inverse);
        CreateView();
    }

    /// <summary>Shows what was drawn at the next vertical blank.</summary>
    public void Present() => swapChain->Present(1, 0).ThrowOnFailure();

    public override void Dispose()
    {
        Com.Release(ref view);
        Com.Release(ref swapChain);
    }

    private void CreateView()
    {
        ID3D11Texture2D* buffer = null;
        try
        {
            Guid textureId = ID3D11Texture2D.IID_Guid;
            swapChain->GetBuffer(0, &textureId, (void**)&buffer);
            ID3D11RenderTargetView* v;
            device.Device->CreateRenderTargetView((ID3D11Resource*)buffer, null, &v);
            view = v;
        }
        finally
        {
            Com.Release(ref buffer);
        }
    }

    private static T* Query<T>(void* from)
        where T : unmanaged, IComIID
    {
        Guid id = T.Guid;
        T* to;
        ((IUnknown*)from)->QueryInterface(&id, (void**)&to).ThrowOnFailure();
        return to;
    }
}
