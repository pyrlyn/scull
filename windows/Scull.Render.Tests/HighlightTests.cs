using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>The highlight colours and their order, as Palette.swift has them, with no device, so every host runs them.</summary>
[TestClass]
public sealed class HighlightTests
{
    [TestMethod]
    public void TheCurrentMatchIsOverTheSelectionOverAMatch()
    {
        // "ab" matches at 0, 3 and 9; the step makes 0 current; 3 to 6 are selected.
        using var terminal = new Terminal(12, 1);
        terminal.Feed("ab ab cd ab"u8);
        terminal.SearchSet("ab", ignoreCase: false);
        terminal.SearchStep(forward: true, out _);
        TestSelection.Select(terminal, 0, 3, 0, 6);
        terminal.Update();
        ReadOnlySpan<Cell> cells = terminal.View.Cells;
        var palette = new Palette();
        const uint Own = 0x123456;

        Assert.AreEqual(palette.CurrentMatch, palette.Highlight(cells[0], Own));
        Assert.AreEqual(palette.Selection, palette.Highlight(cells[3], Own), "a selected match");
        Assert.AreEqual(palette.Selection, palette.Highlight(cells[6], Own));
        Assert.AreEqual(palette.Match, palette.Highlight(cells[9], Own));
        Assert.AreEqual(Own, palette.Highlight(cells[7], Own));
        Assert.IsTrue(cells[0].IsMatch && cells[0].IsCurrentMatch && !cells[0].IsSelected);
    }

    [TestMethod]
    public void TheBuiltInHighlightsAreTheMacOSOnes()
    {
        var palette = new Palette();
        Assert.AreEqual((0x264F78u, 0x6B5A1Au, 0xF2A33Au, 0x000000u),
            (palette.Selection, palette.Match, palette.CurrentMatch, palette.CurrentMatchText));
    }

    [TestMethod]
    public void TheSelectionIsTheAccentTonedDownOverTheBackground()
    {
        var settings = new ConfigSettings { Background = 0x002B36 };
        // 0x78 * 0.45 + 0x2B * 0.55 = 77.65; 0xD4 * 0.45 + 0x36 * 0.55 = 125.1.
        Assert.AreEqual(0x004E7Du, new Palette(settings, accent: 0x0078D4).Selection);
        Assert.AreEqual(0x264F78u, new Palette(settings).Selection, "no accent keeps the built-in one");
    }

    [TestMethod]
    public void BlendingRoundsHalvesAwayFromZeroPerChannel()
    {
        Assert.AreEqual(0x808080u, Palette.Blend(0xFFFFFF, 0x000000, 0.5), "127.5 rounds up, as Swift's rounded()");
        Assert.AreEqual(0xFF0000u, Palette.Blend(0xFF0000, 0x00FF00, 1));
        Assert.AreEqual(0x00FF00u, Palette.Blend(0xFF0000, 0x00FF00, 0));
    }
}
