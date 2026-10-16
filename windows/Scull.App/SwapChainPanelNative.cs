using System.Runtime.InteropServices;
using Microsoft.UI.Xaml.Controls;

namespace Scull.App;

/// <summary>
/// <c>ISwapChainPanelNative</c> of Microsoft.UI.Xaml, which neither the
/// Windows SDK metadata nor the projection carries (research.md §6.1). One
/// method, so a vtable call rather than a declared COM interface.
/// </summary>
internal static unsafe class SwapChainPanelNative
{
    // microsoft.ui.xaml.media.dxinterop.idl; Windows.UI.Xaml's interface of the same name has another IID.
    private static readonly Guid Iid = new("63aad0b8-7c24-40ff-85a8-640d944cc325");

    /// <summary>Shows <paramref name="swapChain"/> (an <c>IDXGISwapChain*</c>) in <paramref name="panel"/>, which keeps its own reference.</summary>
    public static void SetSwapChain(SwapChainPanel panel, nint swapChain)
    {
        nint unknown = ((WinRT.IWinRTObject)panel).NativeObject.ThisPtr;
        Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unknown, in Iid, out nint native));
        try
        {
            // Slot 3: the first method after IUnknown's three.
            var setSwapChain = (delegate* unmanaged[Stdcall]<nint, nint, int>)(*(void***)native)[3];
            Marshal.ThrowExceptionForHR(setSwapChain(native, swapChain));
        }
        finally
        {
            Marshal.Release(native);
        }
    }
}
