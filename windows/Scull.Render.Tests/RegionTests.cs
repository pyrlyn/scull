using System.Runtime.Versioning;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>A pane draws into its own region of a shared target and touches nothing else.</summary>
[TestClass]
[OSCondition(OperatingSystems.Windows)]
[SupportedOSPlatform("windows8.1")]
public sealed class RegionTests
{
    [TestMethod]
    public void ARegionIsFilledAndClippedAndItsNeighbourIsLeftAlone()
    {
        using var device = new RenderDevice(warp: true);
        using var terminal = new Terminal(4, 2);
        using var renderer = new Renderer(device, "Consolas", 16);
        CellMetrics m = renderer.Metrics;
        int width = 4 * m.Width, height = 2 * m.Height;
        using var target = new OffscreenTarget(device, width * 2 + 10, height);

        terminal.Feed("\e[?25l\e[41m  "u8);
        terminal.Update();
        renderer.Absorb(terminal.View);
        var region = new Region(width + 10, 0, width, height);
        renderer.Render(terminal.View, target, region);
        byte[] pixels = target.ReadPixels();

        uint At(int x, int y)
        {
            int i = (y * target.Width + x) * 4;
            return (uint)(pixels[i + 2] << 16 | pixels[i + 1] << 8 | pixels[i]);
        }

        Assert.AreEqual(0xCD0000u, At(region.X + 1, 1), "the grid starts at the region's top-left pixel");
        Assert.AreEqual(0x141414u, At(region.X + 3 * m.Width, 1), "default background fills the region");
        Assert.AreEqual(0x141414u, At(region.X + width - 1, height - 1), "to its last pixel");
        Assert.AreEqual(0u, At(0, 0), "the area to its left was not touched");
        Assert.AreEqual(0u, At(region.X - 1, 1), "not even the gap");
    }
}
