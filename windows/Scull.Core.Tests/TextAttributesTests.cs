using System.Text;

namespace Scull.Core.Tests;

[TestClass]
public sealed class TextAttributesTests
{
    private const uint DefaultForeground = 0xE5E5E5, DefaultBackground = 0x141414;
    private const uint RgbKind = 0x0200_0000;
    private static readonly CellGeometry Geometry = new(10, 20, 0, 0);

    // A resolver as simple as the tests need: true colour as is, anything else the defaults.
    private static (uint Foreground, uint Background) Colors(Style style) =>
        (Rgb(style.Foreground, DefaultForeground), Rgb(style.Background, DefaultBackground));

    private static uint Rgb(uint color, uint fallback) => (color & 0xFF00_0000) == RgbKind ? color & 0xFF_FFFF : fallback;

    private static TextSnapshot Screen(string sgrText)
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed(Encoding.UTF8.GetBytes(sgrText));
        Assert.IsTrue(terminal.Update());
        return TextSnapshot.Capture(terminal, Geometry, new TextLook(Colors, "Cascadia Mono", 11.5));
    }

    private static AttributeResult Get(TextSnapshot snapshot, int start, int end, int attribute) =>
        snapshot.Attributes!.Get(new TextRange(start, end), attribute);

    private static void AreValue(object expected, AttributeResult actual, string message = "")
    {
        Assert.AreEqual(AttributeState.Value, actual.State, message);
        Assert.AreEqual(expected, actual.Value, message);
    }

    [TestMethod]
    public void PlainTextHasThePanesFontTheDefaultColoursAndNoDecoration()
    {
        TextSnapshot snapshot = Screen("plain");

        AreValue("Cascadia Mono", Get(snapshot, 0, 5, AttributeId.FontName));
        AreValue(11.5, Get(snapshot, 0, 5, AttributeId.FontSize));
        AreValue(400, Get(snapshot, 0, 5, AttributeId.FontWeight));
        AreValue(false, Get(snapshot, 0, 5, AttributeId.IsItalic));
        AreValue(false, Get(snapshot, 0, 5, AttributeId.IsReadOnly));
        AreValue(false, Get(snapshot, 0, 5, AttributeId.IsHidden));
        AreValue(0, Get(snapshot, 0, 5, AttributeId.UnderlineStyle));
        AreValue(0, Get(snapshot, 0, 5, AttributeId.StrikethroughStyle));
        AreValue(0, Get(snapshot, 0, 5, AttributeId.OverlineStyle));
        // COLORREF is blue, green, red from the top byte down: 0xE5E5E5 and 0x141414 are greys.
        AreValue(0xE5E5E5, Get(snapshot, 0, 5, AttributeId.ForegroundColor));
        AreValue(0x141414, Get(snapshot, 0, 5, AttributeId.BackgroundColor));
    }

    [TestMethod]
    public void ColoursAreComColorRefsNotRgb()
    {
        // 38;2;10;20;30 is r=10 g=20 b=30, so the COLORREF is 0x00 1E 14 0A.
        TextSnapshot snapshot = Screen("\u001b[38;2;10;20;30;48;2;1;2;3mab");

        AreValue(0x1E140A, Get(snapshot, 0, 2, AttributeId.ForegroundColor));
        AreValue(0x030201, Get(snapshot, 0, 2, AttributeId.BackgroundColor));
    }

    [TestMethod]
    public void BoldItalicUnderlineStrikeOverlineAndHiddenAreReadFromTheCells()
    {
        TextSnapshot snapshot = Screen("\u001b[1;3;4;9;53mx");

        AreValue(700, Get(snapshot, 0, 1, AttributeId.FontWeight));
        AreValue(true, Get(snapshot, 0, 1, AttributeId.IsItalic));
        AreValue(1, Get(snapshot, 0, 1, AttributeId.UnderlineStyle));
        AreValue(1, Get(snapshot, 0, 1, AttributeId.StrikethroughStyle));
        AreValue(1, Get(snapshot, 0, 1, AttributeId.OverlineStyle));
    }

    [TestMethod]
    public void AHiddenCellIsHiddenEvenThoughTheCoreBlanksItsText()
    {
        TextSnapshot snapshot = Screen("a\u001b[8mb\u001b[0mc");

        Assert.AreEqual("a c", snapshot.Text.Text.TrimEnd('\n'), "the core reads a hidden cell as a space");
        AreValue(false, Get(snapshot, 0, 1, AttributeId.IsHidden));
        AreValue(true, Get(snapshot, 1, 2, AttributeId.IsHidden));
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 0, 3, AttributeId.IsHidden).State);
    }

    [TestMethod]
    public void EveryUnderlineKindHasItsLineStyle()
    {
        // UIA TextDecorationLineStyle: single 1, double 3, dot 4, dash 5, wavy 8.
        (string sgr, int style)[] kinds = [("4", 1), ("21", 3), ("4:2", 3), ("4:3", 8), ("4:4", 4), ("4:5", 5)];
        foreach ((string sgr, int style) in kinds)
        {
            AreValue(style, Get(Screen($"\u001b[{sgr}mx"), 0, 1, AttributeId.UnderlineStyle));
        }
    }

    [TestMethod]
    public void ARangeWhoseCellsDifferIsMixedAndTheCellsAloneAreNot()
    {
        // "plain " is plain, "bold" bold, then plain again.
        TextSnapshot snapshot = Screen("plain \u001b[1mbold\u001b[0m end");

        AreValue(400, Get(snapshot, 0, 6, AttributeId.FontWeight));
        AreValue(700, Get(snapshot, 6, 10, AttributeId.FontWeight));
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 0, 10, AttributeId.FontWeight).State);
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 5, 7, AttributeId.FontWeight).State, "one cell either side");
        AreValue("Cascadia Mono", Get(snapshot, 0, 14, AttributeId.FontName));
        AreValue(false, Get(snapshot, 0, 14, AttributeId.IsItalic));
    }

    [TestMethod]
    public void MixedIsPerAttributeSoABoldRunOfOneColourIsStillOneColour()
    {
        TextSnapshot snapshot = Screen("\u001b[38;2;9;9;9mab\u001b[1mcd");

        AreValue(0x090909, Get(snapshot, 0, 4, AttributeId.ForegroundColor));
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 0, 4, AttributeId.FontWeight).State);
    }

    [TestMethod]
    public void ARangeAcrossRowsReadsEveryRow()
    {
        TextSnapshot snapshot = Screen("ab\r\n\u001b[3mcd");

        Assert.AreEqual("ab\ncd\n\n", snapshot.Text.Text);
        AreValue(false, Get(snapshot, 0, 2, AttributeId.IsItalic));
        AreValue(true, Get(snapshot, 3, 5, AttributeId.IsItalic));
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 0, 5, AttributeId.IsItalic).State);
        AreValue(false, Get(snapshot, 0, 3, AttributeId.IsItalic));
    }

    [TestMethod]
    public void AWideCharacterTakesItsOwnCellsStyle()
    {
        TextSnapshot snapshot = Screen("a\u001b[1m世\u001b[0mb");

        AreValue(700, Get(snapshot, 1, 2, AttributeId.FontWeight));
        AreValue(400, Get(snapshot, 2, 3, AttributeId.FontWeight));
        Assert.AreEqual(AttributeState.Mixed, Get(snapshot, 0, 3, AttributeId.FontWeight).State);
    }

    [TestMethod]
    public void AnEmptyRangeReadsTheCharacterItSitsOn()
    {
        TextSnapshot snapshot = Screen("a\u001b[1mbc");

        AreValue(400, Get(snapshot, 0, 0, AttributeId.FontWeight));
        AreValue(700, Get(snapshot, 1, 1, AttributeId.FontWeight));
        AreValue(700, Get(snapshot, 3, 3, AttributeId.FontWeight), "past the row's last character it is that character");
        AreValue(400, Get(snapshot, 99, 99, AttributeId.FontWeight), "past the text it is the last row, which is blank");
    }

    [TestMethod]
    public void ABlankRowAnswersTheDefaultStyle()
    {
        TextSnapshot snapshot = Screen("\u001b[1mx\r\n");

        // The second row has no characters; the empty range there is the default style.
        AreValue(400, Get(snapshot, 2, 2, AttributeId.FontWeight));
        AreValue("Cascadia Mono", Get(snapshot, 2, 2, AttributeId.FontName));
    }

    [TestMethod]
    public void AttributesTheTerminalDoesNotReportAreUnsupported()
    {
        TextSnapshot snapshot = Screen("x");

        foreach (int id in new[] { 40000, 40002, 40009, 40029, 40025, 99999, 0 })
        {
            Assert.AreEqual(AttributeState.Unsupported, Get(snapshot, 0, 1, id).State, id.ToString(System.Globalization.CultureInfo.InvariantCulture));
        }
    }

    [TestMethod]
    public void FindReturnsTheFirstRunOfTheValueCutToTheRange()
    {
        // 0123456789...: "ab" plain, "cd" bold, "ef" plain, "gh" bold.
        TextSnapshot snapshot = Screen("ab\u001b[1mcd\u001b[0mef\u001b[1mgh");
        TextAttributes attributes = snapshot.Attributes!;
        TextRange all = new(0, 8);

        Assert.AreEqual(new TextRange(2, 4), attributes.Find(all, AttributeId.FontWeight, 700, backward: false));
        Assert.AreEqual(new TextRange(6, 8), attributes.Find(all, AttributeId.FontWeight, 700, backward: true));
        Assert.AreEqual(new TextRange(0, 2), attributes.Find(all, AttributeId.FontWeight, 400, backward: false));
        Assert.AreEqual(new TextRange(4, 6), attributes.Find(new TextRange(3, 8), AttributeId.FontWeight, 400, backward: false), "a run starting before the range is cut at its start");
        Assert.AreEqual(new TextRange(3, 4), attributes.Find(new TextRange(3, 5), AttributeId.FontWeight, 700, backward: false));
    }

    [TestMethod]
    public void FindMatchesNumbersWhateverTypeTheClientBoxed()
    {
        TextSnapshot snapshot = Screen("a\u001b[38;2;10;20;30mb");
        TextAttributes attributes = snapshot.Attributes!;
        TextRange all = new(0, 2);

        Assert.AreEqual(new TextRange(1, 2), attributes.Find(all, AttributeId.ForegroundColor, 0x1E140A, false));
        Assert.AreEqual(new TextRange(1, 2), attributes.Find(all, AttributeId.ForegroundColor, (long)0x1E140A, false));
        Assert.AreEqual(new TextRange(1, 2), attributes.Find(all, AttributeId.ForegroundColor, (double)0x1E140A, false));
        Assert.AreEqual(new TextRange(0, 2), attributes.Find(all, AttributeId.FontName, "Cascadia Mono", false));
        Assert.IsNull(attributes.Find(all, AttributeId.FontName, 12, false), "a string attribute never equals a number");
        Assert.IsNull(attributes.Find(all, AttributeId.IsItalic, "yes", false));
    }

    [TestMethod]
    public void FindReturnsNullForNothingAnEmptyRangeAndAnUnsupportedAttribute()
    {
        TextSnapshot snapshot = Screen("abc");
        TextAttributes attributes = snapshot.Attributes!;

        Assert.IsNull(attributes.Find(new TextRange(0, 3), AttributeId.IsItalic, true, false));
        Assert.IsNull(attributes.Find(new TextRange(1, 1), AttributeId.IsItalic, false, false));
        Assert.IsNull(attributes.Find(new TextRange(0, 3), 40000, 1, false));
        Assert.ThrowsExactly<ArgumentNullException>(() => attributes.Find(new TextRange(0, 3), AttributeId.IsItalic, null!, false));
    }

    [TestMethod]
    public void ARunContinuesAcrossTheNewlineBetweenRows()
    {
        TextSnapshot snapshot = Screen("\u001b[3mab\r\ncd\u001b[0m\r\nef");

        Assert.AreEqual(new TextRange(0, 5), snapshot.Attributes!.Find(snapshot.Document, AttributeId.IsItalic, true, false));
    }

    [TestMethod]
    public void ASnapshotWithoutALookHasNoAttributes()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("x"u8);
        terminal.Update();

        Assert.IsNull(TextSnapshot.Capture(terminal, Geometry).Attributes);
    }
}
