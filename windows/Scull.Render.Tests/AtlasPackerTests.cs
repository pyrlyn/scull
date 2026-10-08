using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>The atlas's bookkeeping and the palette: pure logic, so these run on every host.</summary>
[TestClass]
public sealed class AtlasPackerTests
{
    private static readonly GlyphKey A = new(0, 1), B = new(0, 2), C = new(0, 3), D = new(0, 4), E = new(0, 5);

    // 32 texels square: two shelves of 12 (11 rounded up), each two 15-wide glyphs across.
    private static AtlasPacker Full(ulong first, ulong second)
    {
        var packer = new AtlasPacker(size: 32, maxSize: 64);
        foreach ((GlyphKey key, ulong frame) in new[] { (A, first), (B, first), (C, second), (D, second) })
        {
            Assert.IsNotNull(packer.Place(key, 15, 11, 0, 0, false, frame));
        }
        return packer;
    }

    [TestMethod]
    public void APlacedGlyphIsFoundAgainAsOneEntry()
    {
        var packer = new AtlasPacker();
        AtlasEntry placed = packer.Place(A, 8, 14, -1, 12, false, 1)!.Value;

        Assert.IsTrue(packer.TryGet(A, 2, out AtlasEntry found));
        Assert.AreEqual(placed, found);
        Assert.AreEqual(1, packer.Count);
        Assert.IsFalse(packer.TryGet(B, 2, out _));
    }

    [TestMethod]
    public void GlyphsOfOneHeightShareAShelfWithoutOverlapping()
    {
        var packer = new AtlasPacker();
        AtlasEntry a = packer.Place(A, 10, 12, 0, 0, false, 1)!.Value;
        AtlasEntry b = packer.Place(B, 10, 11, 0, 0, false, 1)!.Value;
        AtlasEntry tall = packer.Place(C, 10, 40, 0, 0, false, 1)!.Value;

        Assert.AreEqual((a.Y, a.Shelf), (b.Y, b.Shelf));
        Assert.IsGreaterThanOrEqualTo(a.X + a.Width, (int)b.X);
        Assert.AreNotEqual(a.Shelf, tall.Shelf, "a much taller glyph opens its own shelf");
    }

    [TestMethod]
    public void AFullAtlasEvictsTheShelfDrawnLeastRecently()
    {
        AtlasPacker packer = Full(first: 1, second: 2);
        Assert.IsTrue(packer.TryGet(C, 3, out _), "drawing C keeps its shelf");

        AtlasEntry e = packer.Place(E, 15, 11, 0, 0, false, 3)!.Value;

        Assert.AreEqual((0, 0), (e.X, e.Y), "E took the first shelf");
        Assert.IsFalse(packer.TryGet(A, 3, out _));
        Assert.IsFalse(packer.TryGet(B, 3, out _));
        Assert.IsTrue(packer.TryGet(D, 3, out _));
        Assert.AreEqual(0, packer.Generation);
    }

    [TestMethod]
    public void AnAtlasDrawnWhollyThisFrameGrowsAndStartsOver()
    {
        AtlasPacker packer = Full(first: 1, second: 1);

        Assert.IsNotNull(packer.Place(E, 15, 11, 0, 0, false, 1));

        Assert.AreEqual(1, packer.Generation);
        Assert.AreEqual(64, packer.Size);
        Assert.AreEqual(1, packer.Count, "every entry from before the growth is gone");
        Assert.IsFalse(packer.TryGet(A, 1, out _));
    }

    [TestMethod]
    public void AtTheLargestSizeAGlyphThatFitsNowhereIsRefused()
    {
        AtlasPacker packer = Full(first: 1, second: 1);
        Assert.IsNotNull(packer.Place(E, 15, 11, 0, 0, false, 1));

        Assert.IsNull(packer.Place(new GlyphKey(1, 1), 70, 11, 0, 0, false, 1), "wider than the largest atlas");
        Assert.AreEqual(1, packer.Generation);
    }

    [TestMethod]
    public void BlanksAreRememberedWithoutTexels()
    {
        var packer = new AtlasPacker();
        packer.AddEmpty(A);

        Assert.IsTrue(packer.TryGet(A, 1, out AtlasEntry entry));
        Assert.IsTrue(entry.IsEmpty);
    }

    [TestMethod]
    public void PaletteResolvesCoreColours()
    {
        var palette = new Palette();

        Assert.AreEqual(0xCD0000u, palette.Rgb(0x0100_0001, 7), "indexed 1, xterm red");
        Assert.AreEqual(0xFF0000u, palette.Rgb(0x0100_00C4, 7), "indexed 196 from the cube");
        Assert.AreEqual(0x080808u, palette.Rgb(0x0100_00E8, 7), "indexed 232, the first grey");
        Assert.AreEqual(0x123456u, palette.Rgb(0x0212_3456, 7));
        Assert.AreEqual(7u, palette.Rgb(0, 7), "the default takes the fallback");
        Assert.AreEqual((palette.Background, palette.Foreground), palette.Colors(InverseStyle()));
    }

    private static Style InverseStyle()
    {
        using var terminal = new Terminal(4, 1);
        terminal.Feed("\e[7mx"u8);
        terminal.Update();
        FrameView view = terminal.View;
        return view.StyleAt(view.Cells[0].Style);
    }
}
