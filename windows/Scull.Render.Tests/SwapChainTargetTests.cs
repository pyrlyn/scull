using System.Runtime.Versioning;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>
/// The composition swap chain the shell hands its panel, on WARP with no
/// panel: a frame draws and presents, and a resize keeps it drawing.
/// </summary>
[TestClass]
[OSCondition(OperatingSystems.Windows)]
[SupportedOSPlatform("windows8.1")]
public sealed class SwapChainTargetTests
{
    [TestMethod]
    public void AFrameDrawsAndPresentsBeforeAndAfterAResize()
    {
        using var device = new RenderDevice(warp: true);
        using var terminal = new Terminal(10, 2);
        using var renderer = new Renderer(device, "Consolas", 16);
        using var target = new SwapChainTarget(device, 10 * renderer.Metrics.Width, 2 * renderer.Metrics.Height);
        Assert.AreNotEqual(0, target.NativeSwapChain);

        terminal.Feed("hi"u8);
        terminal.Update();
        renderer.Absorb(terminal.View);
        renderer.Render(terminal.View, target);
        target.Present();

        terminal.Resize(20, 4);
        terminal.Update();
        renderer.Absorb(terminal.View);
        target.Resize(20 * renderer.Metrics.Width, 4 * renderer.Metrics.Height, 1.5f, 1.5f);
        Assert.AreEqual((20 * renderer.Metrics.Width, 4 * renderer.Metrics.Height), (target.Width, target.Height));
        renderer.Render(terminal.View, target);
        target.Present();
    }

    [TestMethod]
    public void AnEmptySizeStillMakesABuffer()
    {
        using var device = new RenderDevice(warp: true);
        using var target = new SwapChainTarget(device, 0, 0);
        Assert.AreEqual((1, 1), (target.Width, target.Height));
        target.Resize(0, -3, 1, 1);
        Assert.AreEqual((1, 1), (target.Width, target.Height));
    }
}
