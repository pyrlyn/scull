using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>Sprite shapes as plain coverage, with no device, so they run on every host.</summary>
[TestClass]
public sealed class SpriteTests
{
    // An even cell with a two-pixel line: the middle is row 10 and column 5,
    // so a light bar covers rows 9-10 and columns 4-5.
    private static readonly CellMetrics Cell = new(Width: 10, Height: 20, Baseline: 15, Underline: 17, Strike: 10, Thickness: 2);
    private readonly SpriteCanvas canvas = new();

    /// <summary>The sprite's coverage at every pixel of one cell, placed as the renderer places it.</summary>
    private byte[,] Draw(GlyphKey key, CellMetrics? metrics = null)
    {
        CellMetrics m = metrics ?? Cell;
        var grid = new byte[m.Height, m.Width];
        if (canvas.Draw(key, m, out GlyphBitmap b))
        {
            for (int y = 0; y < b.Height; y++)
            {
                for (int x = 0; x < b.Width; x++)
                {
                    grid[m.Baseline - b.Top + y, b.Left + x] = b.Pixels[(y * b.Width + x) * 4 + 3];
                }
            }
        }
        return grid;
    }

    private byte[,] Draw(uint codePoint) => Draw(Sprites.Key(codePoint));

    private static IEnumerable<(int Y, int X)> Pixels(int fromY, int toY, int fromX, int toX) =>
        from y in Enumerable.Range(fromY, toY - fromY) from x in Enumerable.Range(fromX, toX - fromX) select (y, x);

    private static void Covered(byte[,] grid, int fromY, int toY, int fromX, int toX, byte value = 255)
    {
        foreach ((int y, int x) in Pixels(fromY, toY, fromX, toX))
        {
            Assert.AreEqual(value, grid[y, x], $"at row {y}, column {x}");
        }
    }

    private static int Count(byte[,] grid) => grid.Cast<byte>().Count(c => c != 0);

    [TestMethod]
    public void SpritesCoverBoxDrawingBlocksAndPowerline()
    {
        foreach (uint code in new uint[] { 0x2500, 0x257F, 0x2580, 0x259F, 0xE0B0, 0xE0BF })
        {
            Assert.IsTrue(Sprites.Covers(code), $"U+{code:X4}");
        }
        foreach (uint code in new uint[] { 'a', 0x24FF, 0x25A0, 0xE0AF, 0xE0C0 })
        {
            Assert.IsFalse(Sprites.Covers(code), $"U+{code:X4}");
        }
        GlyphKey curly = Sprites.UnderlineKey(Underline.Curly);
        Assert.AreEqual(Sprites.Face, curly.Face);
        Assert.IsLessThan(0x2500, (int)curly.Glyph, "underline kinds never collide with a character");
    }

    [TestMethod]
    public void LightLinesReachEveryEdgeSoNeighboursJoin()
    {
        byte[,] horizontal = Draw(0x2500);
        Covered(horizontal, 9, 11, 0, 10);
        Assert.AreEqual(20, Count(horizontal), "nothing outside the bar");

        byte[,] vertical = Draw(0x2502);
        Covered(vertical, 0, 20, 4, 6);
        Assert.AreEqual(40, Count(vertical));
    }

    [TestMethod]
    public void AHeavyLineIsTwiceAsThick()
    {
        byte[,] grid = Draw(0x2501);
        Covered(grid, 8, 12, 0, 10);
        Assert.AreEqual(40, Count(grid));
    }

    [TestMethod]
    public void ACrossAndACornerMeetInTheMiddle()
    {
        byte[,] cross = Draw(0x253C);
        Covered(cross, 9, 11, 0, 10);
        Covered(cross, 0, 20, 4, 6);
        Assert.AreEqual(0, cross[0, 0]);

        byte[,] corner = Draw(0x250C);
        Covered(corner, 9, 11, 4, 10);
        Covered(corner, 9, 20, 4, 6);
        Assert.AreEqual(0, corner[9, 0], "no arm to the left");
        Assert.AreEqual(0, corner[0, 4], "no arm up");
    }

    [TestMethod]
    public void ADoubleLineIsTwoLinesWithAGap()
    {
        byte[,] grid = Draw(0x2550);
        Covered(grid, 7, 9, 0, 10);
        Covered(grid, 11, 13, 0, 10);
        Covered(grid, 9, 11, 0, 10, value: 0);
    }

    [TestMethod]
    public void DashesLeaveGapsAtTheirEnds()
    {
        byte[,] grid = Draw(0x2504);
        foreach (int x in new[] { 0, 3, 6 })
        {
            Assert.AreEqual(0, grid[9, x], $"gap at column {x}");
        }
        foreach (int x in new[] { 1, 4, 7 })
        {
            Assert.AreEqual(255, grid[9, x], $"dash at column {x}");
        }
    }

    [TestMethod]
    public void AnArcRunsFromTheBottomEdgeToTheRightEdge()
    {
        byte[,] grid = Draw(0x256D);
        Assert.IsGreaterThan(0, grid[19, 4] + grid[19, 5], "reaches the bottom edge on the vertical line");
        Assert.IsGreaterThan(0, grid[9, 9] + grid[10, 9], "reaches the right edge on the horizontal line");
        Assert.AreEqual(0, grid[0, 4], "not the top");
        Assert.AreEqual(0, grid[9, 0], "not the left");
    }

    [TestMethod]
    public void DiagonalsJoinOppositeCorners()
    {
        byte[,] rising = Draw(0x2571);
        Assert.IsGreaterThan(0, (int)rising[0, 9]);
        Assert.IsGreaterThan(0, (int)rising[19, 0]);
        Assert.AreEqual(0, rising[0, 0]);

        byte[,] both = Draw(0x2573);
        Assert.IsGreaterThan(0, (int)both[0, 0]);
        Assert.IsGreaterThan(0, (int)both[0, 9]);
    }

    [TestMethod]
    public void BlocksFillTheirShareOfTheCell()
    {
        Covered(Draw(0x2588), 0, 20, 0, 10);

        byte[,] upper = Draw(0x2580);
        Covered(upper, 0, 10, 0, 10);
        Covered(upper, 10, 20, 0, 10, value: 0);

        byte[,] lowerEighth = Draw(0x2581);
        Covered(lowerEighth, 18, 20, 0, 10);
        Assert.AreEqual(20, Count(lowerEighth));

        byte[,] leftEighth = Draw(0x258F);
        Covered(leftEighth, 0, 20, 0, 1);
        Assert.AreEqual(20, Count(leftEighth));

        // Upper left, upper right and lower left.
        byte[,] quadrants = Draw(0x259B);
        Covered(quadrants, 0, 10, 0, 10);
        Covered(quadrants, 10, 20, 0, 5);
        Covered(quadrants, 10, 20, 5, 10, value: 0);
    }

    [TestMethod]
    public void ShadesAreEvenCoverage()
    {
        Covered(Draw(0x2591), 0, 20, 0, 10, value: 64);
        Covered(Draw(0x2592), 0, 20, 0, 10, value: 128);
        Covered(Draw(0x2593), 0, 20, 0, 10, value: 191);
    }

    [TestMethod]
    public void SolidPowerlineShapesReachTheirEdgesAndOutlinesStayHollow()
    {
        byte[,] arrow = Draw(0xE0B0);
        Covered(arrow, 2, 18, 0, 1);
        Assert.IsGreaterThan(0, (int)arrow[10, 9], "the tip");
        Assert.AreEqual(0, arrow[0, 9]);
        Assert.AreEqual(0, arrow[19, 9]);

        byte[,] outline = Draw(0xE0B1);
        Assert.AreEqual(0, outline[10, 1], "inside the outline");
        Assert.IsGreaterThan(0, (int)outline[5, 5], "on the outline");

        byte[,] round = Draw(0xE0B4);
        Covered(round, 2, 18, 0, 1);
        Assert.IsGreaterThan(0, (int)round[10, 9]);
        Assert.AreEqual(0, round[0, 9], "the corner the curve cuts off");
    }

    [TestMethod]
    public void EachUnderlineKindHasItsShape()
    {
        byte[,] single = Draw(Sprites.UnderlineKey(Underline.SingleLine));
        Covered(single, 17, 19, 0, 10);
        Assert.AreEqual(20, Count(single));

        byte[,] twice = Draw(Sprites.UnderlineKey(Underline.DoubleLine));
        Covered(twice, 14, 16, 0, 10);
        Covered(twice, 18, 20, 0, 10);
        Covered(twice, 16, 18, 0, 10, value: 0);

        byte[,] curly = Draw(Sprites.UnderlineKey(Underline.Curly));
        double Centre(int x) => Enumerable.Range(0, 20).Sum(y => y * curly[y, x]) / (double)Enumerable.Range(0, 20).Sum(y => curly[y, x]);
        Assert.IsLessThan(Centre(7), Centre(2), "the wave rises in the first half and falls in the second");
        Assert.IsGreaterThan(0, Enumerable.Range(0, 20).Sum(y => curly[y, 0] + curly[y, 9]), "both ends drawn, so cells join");

        byte[,] dotted = Draw(Sprites.UnderlineKey(Underline.Dotted));
        Assert.AreEqual(255, dotted[17, 0]);
        Assert.AreEqual(0, dotted[17, 2]);
        Assert.AreEqual(255, dotted[17, 4]);

        byte[,] dashed = Draw(Sprites.UnderlineKey(Underline.Dashed));
        Covered(dashed, 17, 19, 2, 9);
        Assert.AreEqual(0, dashed[17, 0]);
        Assert.AreEqual(0, dashed[17, 9]);
    }

    [TestMethod]
    public void ABitmapIsCroppedToItsInkAndKeepsItsPlace()
    {
        Assert.IsTrue(canvas.Draw(Sprites.Key(0x2500), Cell, out GlyphBitmap b));
        Assert.AreEqual((0, 10, 2), (b.Left, b.Width, b.Height));
        Assert.AreEqual(Cell.Baseline - 9, b.Top, "the top row is row 9 of the cell");
        Assert.IsFalse(b.IsColor);
    }

    [TestMethod]
    public void OnlySpriteKeysDraw()
    {
        Assert.IsFalse(canvas.Draw(new GlyphKey(0, 0x2500), Cell, out _));
    }

    [TestMethod]
    public void TinyAndLargeCellsDrawEverySpriteWithoutThrowing()
    {
        var codes = Enumerable.Range(0x2500, 0xA0).Concat(Enumerable.Range(0xE0B0, 0x10)).Select(c => Sprites.Key((uint)c))
            .Concat(Enumerable.Range(1, 5).Select(k => Sprites.UnderlineKey((Underline)k)));
        foreach (CellMetrics m in new CellMetrics[] { new(1, 1, 1, 1, 0, 1), new(3, 2, 1, 2, 1, 3), new(41, 87, 66, 75, 44, 5) })
        {
            foreach (GlyphKey key in codes)
            {
                if (canvas.Draw(key, m, out GlyphBitmap b))
                {
                    Assert.IsTrue(b.Left >= 0 && b.Left + b.Width <= m.Width, $"U+{key.Glyph:X4} stays in the cell's columns");
                    Assert.IsTrue(m.Baseline - b.Top >= 0 && m.Baseline - b.Top + b.Height <= m.Height, $"U+{key.Glyph:X4} stays in its rows");
                }
            }
        }
    }
}
