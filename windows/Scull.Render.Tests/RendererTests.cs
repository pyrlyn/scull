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
        public OffscreenTarget Target;
        private readonly ushort cols, rows;

        public Scene(ushort cols = 12, ushort rows = 3)
        {
            (this.cols, this.rows) = (cols, rows);
            Terminal = new Terminal(cols, rows);
            // Consolas ships with every Windows, Server included.
            Renderer = new Renderer(Device, "Consolas", 16);
            Target = new OffscreenTarget(Device, cols * Metrics.Width, rows * Metrics.Height);
        }

        public CellMetrics Metrics => Renderer.Metrics;

        /// <summary>Another font, and a target sized for its cells.</summary>
        public void SetFont(string family, float size)
        {
            Renderer.SetFont(family, size);
            Target.Dispose();
            Target = new OffscreenTarget(Device, cols * Metrics.Width, rows * Metrics.Height);
        }

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
    public void TheHoveredLinkIsUnderlined()
    {
        using var scene = new Scene();
        byte[] plain = scene.Draw("\e[?25l\e]8;;https://example.com\e\\  \e]8;;\e\\  "u8);
        CellMetrics m = scene.Metrics;
        Assert.AreEqual(Background, scene.Pixel(plain, 1, m.Underline), "not before it is hovered");

        scene.Terminal.HoverLink(scene.Terminal.LinkAt(0, 0));
        byte[] pixels = scene.Draw(default);
        for (int x = 0; x < 2 * m.Width; x++)
        {
            Assert.AreEqual(Foreground, scene.Pixel(pixels, x, m.Underline), $"underline at x {x}");
        }
        Assert.AreEqual(Background, scene.Pixel(pixels, 2 * m.Width + 1, m.Underline), "past the link");
    }

    [TestMethod]
    public void BoxDrawingJoinsAcrossCellsAndRows()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\u2500\u2500\r\n\u2502\r\n\u2502"u8);
        CellMetrics m = scene.Metrics;
        // Where Sprites.cs puts a light bar: centred on the cell's middle pixel.
        int lineY = (int)MathF.Floor(m.Height / 2 - m.Thickness / 2f), lineX = (int)MathF.Floor(m.Width / 2 - m.Thickness / 2f);

        for (int x = 0; x < 2 * m.Width; x++)
        {
            Assert.AreEqual(Foreground, scene.Pixel(pixels, x, lineY), $"horizontal line at x {x}");
        }
        Assert.AreEqual(Foreground, scene.Pixel(pixels, lineX, 2 * m.Height - 1), "the bottom of row 1");
        Assert.AreEqual(Foreground, scene.Pixel(pixels, lineX, 2 * m.Height), "meets the top of row 2");
        Assert.AreEqual(2, scene.Renderer.AtlasEntries, "one entry per sprite");
    }

    [TestMethod]
    public void AFullBlockFillsItsCell()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\u2588"u8);
        Assert.IsTrue(scene.Cell(pixels, 0, 0).All(p => p == Foreground));
    }

    [TestMethod]
    public void ACurlyUnderlineIsNotAStraightLine()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\e[4:3m  "u8);
        CellMetrics m = scene.Metrics;

        int[] rows = [.. Enumerable.Range(0, 2 * m.Width).Select(x => Enumerable.Range(0, m.Height).First(y => scene.Pixel(pixels, x, y) != Background))];
        Assert.IsGreaterThan(0, rows.Max() - rows.Min(), "the wave's top moves with x");
        Assert.AreEqual(rows[m.Width / 4], rows[m.Width + m.Width / 4], "one period per cell");
    }

    [TestMethod]
    public void ACombiningMarkDrawsOverItsBase()
    {
        using var plain = new Scene();
        byte[] e = plain.Draw("\e[?25le"u8);
        using var scene = new Scene();
        byte[] marked = scene.Draw("\e[?25l\e[?2027he\u0301"u8);

        int Ink(Scene s, byte[] pixels, int col) => s.Cell(pixels, 0, col).Count(p => p != Background);
        Assert.IsGreaterThan(Ink(plain, e, 0), Ink(scene, marked, 0), "the accent adds ink to the base cell");
        Assert.AreEqual(0, Ink(scene, marked, 1), "and none to the next cell");
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
    public void ImagesDrawOverAndUnderTheText()
    {
        using var scene = new Scene(cols: 4, rows: 2);
        CellMetrics m = scene.Metrics;
        // The core sizes placements in cells from the cell's pixel size.
        scene.Terminal.Resize(4, 2, (ushort)(4 * m.Width), (ushort)(2 * m.Height));
        // 2 x 2 red pixels scaled to one cell, then the same image under text
        // on the next cell, which keeps showing the text over it.
        string red = "/wAA//8AAP//AAD//wAA/w==";
        scene.Terminal.Feed(System.Text.Encoding.UTF8.GetBytes($"\e[?25l\e_Gf=32,s=2,v=2,a=T,i=1,c=1,r=1;{red}\e\\"));
        byte[] pixels = scene.Draw("\e[1;2H\e_Ga=p,i=1,c=1,r=1,z=-1,C=1\e\\\u2588\e[1;3H\e_Ga=p,i=1,c=1,r=1,z=-1,C=1\e\\"u8);

        Assert.IsTrue(scene.Cell(pixels, 0, 0).All(p => p == 0xFF0000), "the image fills its cell");
        Assert.IsTrue(scene.Cell(pixels, 0, 1).All(p => p == Foreground), "a full block over the image");
        Assert.IsTrue(scene.Cell(pixels, 0, 2).All(p => p == 0xFF0000), "under nothing, the image shows");
        Assert.IsTrue(scene.Cell(pixels, 0, 3).All(p => p == Background));
        Assert.AreEqual(1, scene.Renderer.ImageTextures, "one texture for one image placed three times");

        byte[] cleared = scene.Draw("\e_Ga=d,d=A\e\\"u8);
        Assert.IsTrue(scene.Cell(cleared, 0, 0).All(p => p == Background), "a deleted image is no longer drawn");
        Assert.AreEqual(0, scene.Renderer.ImageTextures, "and its texture is released");
    }

    [TestMethod]
    public void AnImageInAPaneStaysInItsRegion()
    {
        using var scene = new Scene(cols: 4, rows: 2);
        CellMetrics m = scene.Metrics;
        scene.Terminal.Resize(4, 2, (ushort)(4 * m.Width), (ushort)(2 * m.Height));
        scene.Terminal.Feed("\e[?25l\e_Gf=32,s=2,v=2,a=T,i=1,c=2,r=1;/wAA//8AAP//AAD//wAA/w==\e\\"u8);
        scene.Terminal.Update();
        scene.Renderer.Absorb(scene.Terminal.View);
        // A pane one cell in: the image's two cells start at its left edge.
        scene.Renderer.Render(scene.Terminal.View, scene.Target, new Region(m.Width, 0, 3 * m.Width, 2 * m.Height));
        byte[] pixels = scene.Target.ReadPixels();

        Assert.IsTrue(scene.Cell(pixels, 0, 1).Concat(scene.Cell(pixels, 0, 2)).All(p => p == 0xFF0000));
        Assert.IsFalse(scene.Cell(pixels, 0, 0).Contains(0xFF0000u), "nothing left of the pane");
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
    public void NerdFontIconsDrawFromTheBundledSymbolsFont()
    {
        using var scene = new Scene();
        byte[] pixels = scene.Draw("\e[?25l\uF015 \uE0A0 \U000F0001"u8);

        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 0).Count(p => p != Background), "a Font Awesome house");
        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 2).Count(p => p != Background), "the Powerline branch symbol");
        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 4).Count(p => p != Background), "a Material Design icon past the BMP");
    }

    [TestMethod]
    public void AFrameOfCachedIconsAllocatesNothing()
    {
        using var scene = new Scene();
        scene.Draw("\uF015\uE0A0\uE0B0\r\n\uF015\uE0A0\uE0B0"u8);
        scene.Terminal.Feed("\r\uF015\uE0A0\uE0B0\e[1;1H\uF015\uE0A0\uE0B0"u8);
        scene.Terminal.Update();

        long before = GC.GetAllocatedBytesForCurrentThread();
        scene.Renderer.Absorb(scene.Terminal.View);
        scene.Renderer.Render(scene.Terminal.View, scene.Target);
        scene.Renderer.Render(scene.Terminal.View, scene.Target);
        Assert.AreEqual(0, GC.GetAllocatedBytesForCurrentThread() - before);
    }

    // Most of a cell in one colour: a glyph covers well under half of it.
    private static bool Mostly(Scene scene, byte[] pixels, int col, uint color)
    {
        uint[] cell = [.. scene.Cell(pixels, 0, col)];
        return cell.Count(p => p == color) > cell.Length / 2;
    }

    [TestMethod]
    public void MatchesTheSelectionAndTheCurrentMatchPaintTheirCells()
    {
        using var scene = new Scene();
        scene.Draw("\e[?25lab ab cd ab"u8);
        scene.Terminal.SearchSet("ab", ignoreCase: false);
        scene.Terminal.SearchStep(forward: true, out _);
        TestSelection.Select(scene.Terminal, 0, 3, 0, 6);
        Palette palette = scene.Renderer.Palette;
        byte[] pixels = scene.Draw([]);

        Assert.IsTrue(Mostly(scene, pixels, 0, palette.CurrentMatch), "the current match");
        Assert.IsTrue(Mostly(scene, pixels, 3, palette.Selection), "a selected match shows the selection");
        Assert.IsTrue(scene.Cell(pixels, 0, 5).All(p => p == palette.Selection), "a selected blank");
        Assert.IsTrue(Mostly(scene, pixels, 9, palette.Match), "another match");
        Assert.IsTrue(scene.Cell(pixels, 0, 2).All(p => p == Background), "a blank in neither");
        Assert.IsTrue(Mostly(scene, pixels, 7, Background));
    }

    [TestMethod]
    public void TheCurrentMatchsTextIsDarkOnItsBrightBackground()
    {
        using var scene = new Scene();
        scene.Draw("\e[?25lab ab"u8);
        scene.Terminal.SearchSet("ab", ignoreCase: false);
        scene.Terminal.SearchStep(forward: true, out _);
        byte[] pixels = scene.Draw([]);

        static bool Dark(uint p) => (p >> 16 & 0xFF) < 0x40 && (p >> 8 & 0xFF) < 0x40 && (p & 0xFF) < 0x40;
        Assert.IsTrue(scene.Cell(pixels, 0, 0).Any(Dark), "the current match's ink is black");
        Assert.IsFalse(scene.Cell(pixels, 0, 0).Any(p => p == Foreground));
        Assert.IsTrue(scene.Cell(pixels, 0, 3).Any(p => p == Foreground), "another match keeps the foreground");
    }

    [TestMethod]
    public void ANewSearchMovesTheHighlightsAndAnEmptyOneClearsThem()
    {
        using var scene = new Scene();
        scene.Draw("\e[?25lab cd"u8);
        scene.Terminal.SearchSet("ab", ignoreCase: false);
        scene.Draw([]);
        scene.Terminal.SearchSet("cd", ignoreCase: false);
        Palette palette = scene.Renderer.Palette;
        byte[] pixels = scene.Draw([]);
        Assert.IsTrue(Mostly(scene, pixels, 0, Background), "the old match is gone");
        Assert.IsTrue(Mostly(scene, pixels, 3, palette.Match));

        scene.Terminal.SearchSet("", ignoreCase: false);
        pixels = scene.Draw([]);
        Assert.IsTrue(Mostly(scene, pixels, 3, Background));
    }

    [TestMethod]
    public void ANewPaletteRedrawsEveryRow()
    {
        using var scene = new Scene();
        scene.Draw("\e[?25l\u2588"u8);
        scene.Renderer.Palette = new Palette { Foreground = 0x839496, Background = 0x002B36 };
        byte[] pixels = scene.Draw([]);

        Assert.IsTrue(scene.Cell(pixels, 0, 0).All(p => p == 0x839496), "the block in the new foreground");
        Assert.IsTrue(scene.Cell(pixels, 0, 1).All(p => p == 0x002B36), "a blank in the new background");
        Assert.IsTrue(scene.Cell(pixels, 2, 11).All(p => p == 0x002B36), "rows the frame did not touch too");
    }

    [TestMethod]
    public void ALargerFontGrowsTheCellsAndRedrawsItsGlyphs()
    {
        using var scene = new Scene(4, 2);
        byte[] before = scene.Draw("\e[?25l\u2588M"u8);
        CellMetrics small = scene.Metrics;
        int smallM = scene.Cell(before, 0, 1).Count(p => p != Background);

        scene.SetFont("Consolas", 32);
        Assert.AreEqual(0, scene.Renderer.AtlasEntries, "the old size's glyphs are gone");
        Assert.IsGreaterThan(small.Height, scene.Metrics.Height);
        Assert.IsGreaterThan(small.Width, scene.Metrics.Width);
        byte[] pixels = scene.Draw([]);

        Assert.IsTrue(scene.Cell(pixels, 0, 0).All(p => p == Foreground), "the block fills the bigger cell");
        Assert.IsGreaterThan(smallM, scene.Cell(pixels, 0, 1).Count(p => p != Background), "M drawn at the new size");
    }

    [TestMethod]
    public void AMissingFamilyKeepsTheFontInUse()
    {
        using var scene = new Scene();
        scene.Draw("\e[?25lM"u8);
        CellMetrics before = scene.Metrics;

        Assert.ThrowsExactly<ArgumentException>(() => scene.Renderer.SetFont("No Such Scull Font", 32));
        Assert.AreEqual(before, scene.Metrics);
        byte[] pixels = scene.Draw([]);
        Assert.IsGreaterThan(10, scene.Cell(pixels, 0, 0).Count(p => p != Background));
    }

    [TestMethod]
    public void AMissingFontFamilyIsRefused()
    {
        Assert.ThrowsExactly<ArgumentException>(() => new FontFaces("No Such Scull Font", 16f));
    }
}
