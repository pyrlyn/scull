using System.Runtime.Versioning;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>
/// Frames from a real terminal drawn on WARP into an offscreen texture and
/// read back. WARP rasterises the same on every machine, so known pixels can
/// be checked exactly where no glyph is.
/// </summary>
[TestClass]
[OSCondition(OperatingSystems.Windows)]
[SupportedOSPlatform("windows8.1")]
public sealed class RendererTests
{
    private const uint Background = 0x141414, Foreground = 0xE5E5E5;

    private sealed class Scene : IDisposable
    {
        public readonly RenderDevice Device = new(warp: true);
        public readonly Terminal Terminal;
        public readonly Renderer Renderer;
        public readonly OffscreenTarget Target;

        public Scene(ushort cols = 12, ushort rows = 3)
        {
            Terminal = new Terminal(cols, rows);
            // Consolas ships with every Windows, Server included.
            Renderer = new Renderer(Device, "Consolas", 16);
            Target = new OffscreenTarget(Device, cols * Metrics.Width, rows * Metrics.Height);
        }

        public CellMetrics Metrics => Renderer.Metrics;

        public byte[] Draw(ReadOnlySpan<byte> bytes)
        {
            if (!bytes.IsEmpty)
            {
                Terminal.Feed(bytes);
            }
            Terminal.Update();
            Renderer.Absorb(Terminal.View);
            Renderer.Render(Terminal.View, Target);
            return Target.ReadPixels();
        }

        public uint Pixel(byte[] pixels, int x, int y)
        {
            int i = (y * Target.Width + x) * 4;
            return (uint)(pixels[i + 2] << 16 | pixels[i + 1] << 8 | pixels[i]);
        }

        /// <summary>Every pixel of a cell.</summary>
        public IEnumerable<uint> Cell(byte[] pixels, int row, int col)
        {
            for (int y = row * Metrics.Height; y < (row + 1) * Metrics.Height; y++)
            {
                for (int x = col * Metrics.Width; x < (col + 1) * Metrics.Width; x++)
                {
                    yield return Pixel(pixels, x, y);
                }
            }
        }

        public void Dispose()
        {
            Target.Dispose();
            Renderer.Dispose();
            Terminal.Dispose();
            Device.Dispose();
        }
    }

    [TestMethod]
    public void BackgroundsFillTheirCells()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\e[41m  \e[48;2;0;0;238m  \e[0m"u8);

        Assert.IsTrue(scene.Cell(pixels, 0, 0).All(p => p == 0xCD0000), "red, palette index 1");
        Assert.IsTrue(scene.Cell(pixels, 0, 3).All(p => p == 0x0000EE), "an RGB background");
        Assert.IsTrue(scene.Cell(pixels, 0, 4).All(p => p == Background));
        Assert.IsTrue(scene.Cell(pixels, 1, 0).All(p => p == Background));
    }

    [TestMethod]
    public void GlyphsCoverTheirCellsAndBlanksStayClear()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25lMW  \e[32mM"u8);

        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 0).Count(p => p != Background), "M left coverage");
        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 1).Count(p => p != Background));
        Assert.IsTrue(scene.Cell(pixels, 0, 2).All(p => p == Background), "a space draws nothing");
        Assert.IsTrue(scene.Cell(pixels, 0, 4).Contains(0x00CD00u), "green text reaches full colour somewhere");
        Assert.IsFalse(scene.Cell(pixels, 0, 4).Contains(Foreground), "and never the default foreground");
    }

    [TestMethod]
    public void TheCursorIsABlockWhenFocusedAndABoxWhenNot()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("ab"u8);
        Assert.IsTrue(scene.Cell(pixels, 0, 2).All(p => p == Foreground), "the block fills the cell after the text");

        scene.Renderer.Focused = false;
        pixels = scene.Draw(""u8);
        CellMetrics m = scene.Metrics;
        Assert.AreEqual(Foreground, scene.Pixel(pixels, 2 * m.Width, m.Height / 2), "left edge");
        Assert.AreEqual(Background, scene.Pixel(pixels, 2 * m.Width + m.Width / 2, m.Height / 2), "hollow inside");
    }

    [TestMethod]
    public void UnderlineAndStrikeSpanTheirCells()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\e[4m  \e[0m\e[9m  "u8);
        CellMetrics m = scene.Metrics;

        for (int x = 0; x < 2 * m.Width; x++)
        {
            Assert.AreEqual(Foreground, scene.Pixel(pixels, x, m.Underline), $"underline at x {x}");
            Assert.AreEqual(Foreground, scene.Pixel(pixels, 2 * m.Width + x, m.Strike), $"strike at x {x}");
        }
        Assert.AreEqual(Background, scene.Pixel(pixels, 1, m.Strike), "an underlined cell has no strike");
    }

    [TestMethod]
    public void ARepeatedGlyphIsOneAtlasEntry()
    {
        using var scene = new Scene();
        scene.Draw("aaaa"u8);
        Assert.AreEqual(1, scene.Renderer.AtlasEntries);

        scene.Draw("\r\nab a"u8);
        Assert.AreEqual(2, scene.Renderer.AtlasEntries);
    }

    [TestMethod]
    public void RepaintingOnlyTheDamageGivesTheFullPicture()
    {
        using var scene = new Scene(cols: 10, rows: 4);
        byte[] damaged = [];
        foreach (string line in new[] { "one", "\r\n\e[31mtwo", "\r\nthree", "\r\n\e[44mfour", "\r\nfive", "\r\n\e[0msix\e[2;1Hx" })
        {
            damaged = scene.Draw(System.Text.Encoding.UTF8.GetBytes(line));
        }

        using var fresh = new Renderer(scene.Device, "Consolas", 16);
        fresh.Render(scene.Terminal.View, scene.Target);

        CollectionAssert.AreEqual(scene.Target.ReadPixels(), damaged);
    }

    [TestMethod]
    public void AColourGlyphKeepsItsColours()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\U0001F600"u8);

        bool coloured = scene.Cell(pixels, 0, 0).Concat(scene.Cell(pixels, 0, 1)).Any(p =>
        {
            int r = (int)(p >> 16), g = (int)(p >> 8 & 0xFF), b = (int)(p & 0xFF);
            return Math.Abs(r - b) > 64 || Math.Abs(g - b) > 64;
        });
        Assert.IsTrue(coloured, "Segoe UI Emoji's layers are not grey");
    }

    [TestMethod]
    public void AFrameOfCachedGlyphsAllocatesNothing()
    {
        using var scene = new Scene();
        scene.Draw("hello\r\nhello"u8);
        scene.Terminal.Feed("\rhello\e[1;1Hhello"u8);
        scene.Terminal.Update();

        long before = GC.GetAllocatedBytesForCurrentThread();
        scene.Renderer.Absorb(scene.Terminal.View);
        scene.Renderer.Render(scene.Terminal.View, scene.Target);
        scene.Renderer.Render(scene.Terminal.View, scene.Target);
        long allocated = GC.GetAllocatedBytesForCurrentThread() - before;

        Assert.AreEqual(0, allocated);
    }

    [TestMethod]
    public void AMissingFontFamilyIsRefused()
    {
        Assert.ThrowsExactly<ArgumentException>(() => new FontFaces("No Such Scull Font", 16f));
    }
}
