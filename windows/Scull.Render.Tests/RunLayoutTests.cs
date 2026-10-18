using Windows.Win32.Graphics.DirectWrite;

namespace Scull.Render.Tests;

/// <summary>
/// Glyphs laid on the grid from DirectWrite-shaped outputs written by hand,
/// and the shaped-run cache; no device, so every host runs them.
/// </summary>
[TestClass]
public sealed class RunLayoutTests
{
    private static ShapedGlyph[] Lay(string text, int width, ushort[] clusterMap, ushort[] glyphs, float[] advances,
        DWRITE_GLYPH_OFFSET[]? offsets = null, int first = 0)
    {
        var layout = new RunLayout();
        layout.Begin(text, width);
        layout.Place(first, 0, clusterMap, glyphs, advances, offsets ?? new DWRITE_GLYPH_OFFSET[glyphs.Length]);
        return [.. layout.Glyphs];
    }

    private static string Columns(IEnumerable<ShapedGlyph> glyphs) => string.Join(",", glyphs.Select(g => g.Column));

    [TestMethod]
    public void ALigatureIsOneGlyphAtItsFirstColumn()
    {
        ShapedGlyph[] glyphs = Lay("fi", 1, [0, 0], [7], [8]);
        Assert.HasCount(1, glyphs);
        Assert.AreEqual(new ShapedGlyph(new GlyphKey(0, 7), 0, 0, 0), glyphs[0]);
    }

    [TestMethod]
    public void AMarkKeepsItsOffsetFromItsBase()
    {
        // DirectWrite puts e and its accent in one cluster; the accent has no
        // advance and is pulled back over the e.
        ShapedGlyph[] glyphs = Lay("é", 1, [0, 0], [3, 9], [8, 0], [default, new() { advanceOffset = -6, ascenderOffset = 3 }]);
        Assert.AreEqual("0,0", Columns(glyphs));
        Assert.AreEqual(2f, glyphs[1].Dx);
        Assert.AreEqual(3f, glyphs[1].Dy);
    }

    [TestMethod]
    public void AWideFallbackAdvanceStaysOnTheGrid()
    {
        // A fallback face wider than the cell would push b right by its pen.
        ShapedGlyph[] glyphs = Lay("abc", 1, [0, 1, 2], [1, 2, 3], [20, 20, 20]);
        Assert.AreEqual("0,1,2", Columns(glyphs));
        Assert.IsTrue(glyphs.All(g => g.Dx == 0), "each glyph starts at its own column");
    }

    [TestMethod]
    public void WideCharactersTakeTwoColumnsEach()
    {
        ShapedGlyph[] glyphs = Lay("中文", 2, [0, 1], [1, 2], [16, 16]);
        Assert.AreEqual("0,2", Columns(glyphs));
    }

    [TestMethod]
    public void ASurrogatePairIsOneGrapheme()
    {
        ShapedGlyph[] glyphs = Lay("\U0001F600x", 2, [0, 0, 1], [5, 6], [16, 16]);
        Assert.AreEqual("0,2", Columns(glyphs));
    }

    [TestMethod]
    public void SpriteCharactersGoToTheSpriteDrawer()
    {
        ShapedGlyph[] glyphs = Lay("a─b", 1, [0, 1, 2], [1, 2, 3], [8, 8, 8]);
        Assert.AreEqual(1, glyphs.Count(g => g.Key.Face == Sprites.Face));
        Assert.AreEqual(Sprites.Key(0x2500), glyphs.Single(g => g.Column == 1).Key, "the font's glyph for it is dropped");
        Assert.HasCount(3, glyphs);
    }

    [TestMethod]
    public void SpacesHaveNoGlyph()
    {
        Assert.AreEqual("0,2", Columns(Lay("a b", 1, [0, 1, 2], [1, 2, 3], [8, 8, 8])));
    }

    [TestMethod]
    public void APieceLaterInTheRunKeepsItsColumns()
    {
        // The fallback shaped only "b", the run's second character.
        ShapedGlyph[] glyphs = Lay("ab", 1, [0], [4], [8], first: 1);
        Assert.AreEqual("1", Columns(glyphs));
    }

    [TestMethod]
    public void AMalformedClusterMapReadsNoGlyphPastTheEnd()
    {
        ShapedGlyph[] glyphs = Lay("ab", 1, [0, 5], [4], [8]);
        Assert.AreEqual("0", Columns(glyphs));
    }

    [TestMethod]
    public void TheRunCacheStaysBounded()
    {
        var cache = new RunCache<string>(limit: 2);
        foreach (string text in new[] { "a", "b", "c", "d", "e" })
        {
            cache.Add(text, text);
        }
        Assert.IsLessThanOrEqualTo(4, cache.Count, "two generations of two");
    }

    [TestMethod]
    public void ARunStillOnScreenSurvivesTheDrop()
    {
        var cache = new RunCache<string>(limit: 2);
        cache.Add("a", "A");
        cache.Add("b", "B");
        cache.Add("c", "C");
        Assert.IsTrue(cache.TryGet("a", out string? a), "the old generation still answers");
        Assert.AreEqual("A", a);
        cache.Add("d", "D");
        Assert.IsTrue(cache.TryGet("a", out _), "a hit moved it forward");
        Assert.IsFalse(cache.TryGet("b", out _), "untouched, it went with its generation");
    }

    [TestMethod]
    public void ACacheKeyIsFoundBySpan()
    {
        var cache = new RunCache<string>();
        cache.Add("ab", "x");
        Assert.IsTrue(cache.TryGet("xab".AsSpan(1), out _));
    }
}
