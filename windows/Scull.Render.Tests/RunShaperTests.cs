using System.Runtime.Versioning;
using System.Text;

namespace Scull.Render.Tests;

/// <summary>Runs shaped by DirectWrite with the system fallback; Windows only, with fonts every Windows ships.</summary>
[TestClass]
[OSCondition(OperatingSystems.Windows)]
[SupportedOSPlatform("windows8.1")]
public sealed class RunShaperTests
{
    private static ShapedGlyph[] Shape(string family, string text, int face = 0, int width = 1)
    {
        using var fonts = new FontFaces(family, 16);
        using var shaper = new RunShaper(fonts);
        return shaper.Shape(Encoding.UTF8.GetBytes(text), face, width);
    }

    [TestMethod]
    public void TheShaperKeepsLigatures()
    {
        // Calibri's standard ligatures are on by default; a monospace font has none.
        ShapedGlyph[] glyphs = Shape("Calibri", "fi");
        Assert.HasCount(1, glyphs, "f and i are one glyph");
        Assert.AreEqual(0, glyphs[0].Column);
    }

    [TestMethod]
    public void TheShaperFallsBackForCharactersTheFontLacks()
    {
        using var fonts = new FontFaces("Consolas", 16);
        using var shaper = new RunShaper(fonts);
        ShapedGlyph[] glyphs = shaper.Shape("中文"u8, 0, 2);
        Assert.AreEqual("0,2", string.Join(",", glyphs.Select(g => g.Column)));
        Assert.IsTrue(glyphs.All(g => g.Key.Face >= 4 && g.Key.Glyph != 0), "a CJK face from the system fallback");
        Assert.IsTrue(glyphs.All(g => g.Dx == 0), "its wider advance stays in its columns");
    }

    [TestMethod]
    public void TheShaperFallsBackToColourEmoji()
    {
        using var fonts = new FontFaces("Consolas", 16);
        using var shaper = new RunShaper(fonts);
        ShapedGlyph glyph = shaper.Shape("\U0001F600"u8, 0, 2).Single();
        Assert.IsGreaterThanOrEqualTo(4, (int)glyph.Key.Face);
        Assert.IsTrue(fonts.Rasterize(glyph.Key, out GlyphBitmap bitmap) && bitmap.IsColor);
    }

    [TestMethod]
    public void TheShaperPicksBoldAndItalicFaces()
    {
        for (int face = 0; face < 4; face++)
        {
            Assert.AreEqual(face, (int)Shape("Consolas", "a", face).Single().Key.Face);
        }
    }

    [TestMethod]
    public void ACombiningMarkStaysInItsBasesColumn()
    {
        ShapedGlyph[] glyphs = Shape("Consolas", "éx");
        Assert.IsTrue(glyphs.Where(g => g.Column == 0).Any(), "the accented e");
        Assert.AreEqual(1, glyphs.Count(g => g.Column == 1), "x alone in the next column");
    }

    [TestMethod]
    public void TheShaperHandsSpriteCharactersToTheSpriteDrawer()
    {
        ShapedGlyph[] glyphs = Shape("Consolas", "a─b");
        Assert.AreEqual(Sprites.Key(0x2500), glyphs.Single(g => g.Column == 1).Key);
    }

    [TestMethod]
    public void AShapedRunIsCachedAndBounded()
    {
        using var fonts = new FontFaces("Consolas", 16);
        using var shaper = new RunShaper(fonts, limit: 2);
        ShapedGlyph[] first = shaper.Shape("abc"u8, 0, 1);

        long before = GC.GetAllocatedBytesForCurrentThread();
        ShapedGlyph[] again = shaper.Shape("abc"u8, 0, 1);
        Assert.AreEqual(0, GC.GetAllocatedBytesForCurrentThread() - before, "a hit allocates nothing");
        Assert.AreSame(first, again);
        Assert.AreNotSame(first, shaper.Shape("abc"u8, 1, 1), "the face is part of the key");

        foreach (string text in new[] { "d", "e", "f", "g", "h" })
        {
            shaper.Shape(Encoding.UTF8.GetBytes(text), 0, 1);
        }
        Assert.IsLessThanOrEqualTo(4, shaper.Cached);
    }
}
