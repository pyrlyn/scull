namespace Scull.Core.Tests;

[TestClass]
public sealed class ScreenTextTests
{
    private static int One(int col) => 1;

    // 世 and 界 take two columns and the second is a spacer, as the core's cells say.
    private static Func<int, int> Widths(string row)
    {
        var cells = new List<int>();
        foreach (char c in row)
        {
            bool wide = c is '世' or '界';
            cells.Add(wide ? 2 : 1);
            if (wide)
            {
                cells.Add(0);
            }
        }
        return col => col < cells.Count ? cells[col] : 1;
    }

    [TestMethod]
    public void LinesAreRowsAndATrailingNewlineEndsInAnEmptyLine()
    {
        var text = new ScreenText("ab\ncd\n");

        Assert.AreEqual(3, text.LineCount);
        Assert.AreEqual(new TextRange(0, 6), text.Document);
        Assert.AreEqual(new TextRange(0, 3), text.RangeForLine(0));
        Assert.AreEqual(new TextRange(3, 6), text.RangeForLine(1));
        Assert.AreEqual(new TextRange(6, 6), text.RangeForLine(2));
        Assert.IsNull(text.RangeForLine(3));
        Assert.IsNull(text.RangeForLine(-1));
        (int Offset, int Line)[] lines = [(-5, 0), (0, 0), (2, 0), (3, 1), (5, 1), (6, 2), (99, 2)];
        foreach ((int offset, int line) in lines)
        {
            Assert.AreEqual(line, text.LineFor(offset), $"offset {offset}");
        }
        Assert.AreEqual(1, new ScreenText("").LineCount);
    }

    [TestMethod]
    public void TextIsClampedAndCut()
    {
        var text = new ScreenText("hello");

        Assert.AreEqual("ell", text.GetText(new TextRange(1, 4)));
        Assert.AreEqual("ello", text.GetText(new TextRange(1, 99)));
        Assert.AreEqual("he", text.GetText(new TextRange(-3, 2)));
        Assert.AreEqual("el", text.GetText(new TextRange(1, 4), 2));
        Assert.AreEqual("", text.GetText(new TextRange(3, 1)));
        Assert.AreEqual("", text.GetText(new TextRange(1, 4), 0));
        Assert.AreEqual("ell", text.GetText(new TextRange(1, 4), int.MaxValue));
    }

    [TestMethod]
    public void AWideCharacterTakesItsColumnsAndACombiningMarkNone()
    {
        const string row = "a世b";
        var text = new ScreenText(row);
        Func<int, int> width = Widths(row);

        CollectionAssert.AreEqual(new[] { (0, 0), (1, 1), (2, 3), (3, 4) }, text.Columns(0, width).ToArray());
        Assert.AreEqual(1, text.OffsetOf(0, 2, width), "the second column of 世 is 世");
        Assert.AreEqual(2, text.OffsetOf(0, 3, width));
        Assert.AreEqual(3, text.OffsetOf(0, 40, width), "past the end is the end");

        var combined = new ScreenText("éx\n😀a");
        CollectionAssert.AreEqual(new[] { (0, 0), (2, 1), (3, 2) }, combined.Columns(0, One).ToArray());
        CollectionAssert.AreEqual(new[] { (4, 0), (6, 2), (7, 3) }, combined.Columns(1, c => c == 0 ? 2 : 1).ToArray());
        Assert.AreEqual(0, combined.Columns(5, One).Count);
        Assert.AreEqual(combined.Length, combined.OffsetOf(5, 0, One));
    }

    [TestMethod]
    public void RangesBecomeOneSpanPerRow()
    {
        var text = new ScreenText("abc\ndef\nghi");

        CollectionAssert.AreEqual(
            new[] { new LineSpan(0, 1, 3), new LineSpan(1, 0, 3), new LineSpan(2, 0, 2) },
            text.LineSpans(new TextRange(1, 10), One).ToArray());
        CollectionAssert.AreEqual(new[] { new LineSpan(0, 0, 3) }, text.LineSpans(new TextRange(0, 4), One).ToArray(), "the newline belongs to its row");
        CollectionAssert.AreEqual(new[] { new LineSpan(0, 1, 2) }, text.LineSpans(new TextRange(1, 1), One).ToArray(), "an empty range is one cell");
        CollectionAssert.AreEqual(new[] { new LineSpan(0, 3, 4) }, text.LineSpans(new TextRange(3, 4), One).ToArray(), "a bare newline is the cell after the row");
        CollectionAssert.AreEqual(new[] { new LineSpan(2, 3, 4) }, text.LineSpans(new TextRange(99, 99), One).ToArray());

        const string row = "a世界";
        CollectionAssert.AreEqual(
            new[] { new LineSpan(0, 1, 5) },
            new ScreenText(row).LineSpans(new TextRange(1, 3), Widths(row)).ToArray());
    }

    [TestMethod]
    [DataRow("aéb", 1, 1, TextUnit.Character, 1, 3)]
    [DataRow("aéb", 2, 2, TextUnit.Character, 1, 3)] // inside a cluster
    [DataRow("aéb", 1, 2, TextUnit.Character, 1, 3)]
    [DataRow("aéb", 4, 4, TextUnit.Character, 3, 4)] // the caret after the text
    [DataRow("aéb", 1, 3, TextUnit.Character, 1, 3)] // already whole
    [DataRow("foo  bar, baz\nqux", 6, 6, TextUnit.Word, 5, 8)]
    [DataRow("foo  bar, baz\nqux", 0, 0, TextUnit.Word, 0, 5)] // the blanks after a word are part of it
    [DataRow("foo  bar, baz\nqux", 9, 9, TextUnit.Word, 8, 10)] // punctuation is a word of its own
    [DataRow("foo  bar, baz\nqux", 13, 13, TextUnit.Word, 13, 14)] // so is a newline
    [DataRow("foo  bar, baz\nqux", 1, 6, TextUnit.Word, 0, 8)]
    [DataRow("foo  bar, baz\nqux", 17, 17, TextUnit.Word, 14, 17)]
    [DataRow("ab\ncd", 1, 1, TextUnit.Line, 0, 3)]
    [DataRow("ab\ncd", 3, 3, TextUnit.Paragraph, 3, 5)]
    [DataRow("ab\ncd", 1, 4, TextUnit.Line, 0, 5)]
    [DataRow("ab\ncd", 1, 1, TextUnit.Document, 0, 5)]
    [DataRow("ab\ncd", 4, 4, TextUnit.Page, 0, 5)]
    [DataRow("ab\ncd", 2, 2, TextUnit.Format, 0, 5)]
    [DataRow("", 0, 0, TextUnit.Word, 0, 0)]
    [DataRow("", 5, 9, TextUnit.Line, 0, 0)]
    public void ExpandingTakesTheEnclosingUnits(string text, int start, int end, TextUnit unit, int from, int to) =>
        Assert.AreEqual(new TextRange(from, to), new ScreenText(text).ExpandToEnclosingUnit(new TextRange(start, end), unit));

    [TestMethod]
    [DataRow("a.b", new[] { 0, 1, 2, 3 })]
    [DataRow("foo_bar1", new[] { 0, 8 })]
    [DataRow("x=1", new[] { 0, 1, 2, 3 })]
    [DataRow("--flag", new[] { 0, 2, 6 })]
    [DataRow("  ab", new[] { 0, 2, 4 })]
    [DataRow("日本語 ok", new[] { 0, 1, 2, 4, 6 })]
    [DataRow("ターミナル", new[] { 0, 1, 2, 3, 4, 5 })]
    [DataRow("a\n\nb", new[] { 0, 1, 2, 3, 4 })]
    [DataRow("é́e", new[] { 0, 3 })] // marks stay with their letter
    [DataRow("𠀀a", new[] { 0, 2, 3 })] // an ideograph past the BMP
    public void WordsBreakWhereAScreenReaderWouldPause(string text, int[] boundaries)
    {
        var screen = new ScreenText(text);
        var found = new List<int> { 0 };
        for (TextRange word = screen.ExpandToEnclosingUnit(new TextRange(0, 0), TextUnit.Word); word.End < screen.Length; )
        {
            found.Add(word.End);
            word = screen.ExpandToEnclosingUnit(new TextRange(word.End, word.End), TextUnit.Word);
        }
        found.Add(screen.Length);
        CollectionAssert.AreEqual(boundaries, found.Distinct().ToArray());
    }

    [TestMethod]
    public void ALoneSurrogateIsAUnitNotACrash()
    {
        var text = new ScreenText(new string(['\uD800', 'x']));

        Assert.AreEqual(new TextRange(0, 1), text.ExpandToEnclosingUnit(new TextRange(0, 0), TextUnit.Word));
        Assert.AreEqual(new TextRange(1, 2), text.Move(new TextRange(0, 0), TextUnit.Word, 1, out _));
    }

    [TestMethod]
    [DataRow(0, 0, 1, 4, 8, 1)]
    [DataRow(0, 0, 5, 8, 13, 2)] // stops at the last word
    [DataRow(9, 9, -1, 4, 8, -1)]
    [DataRow(9, 9, -9, 0, 4, -2)]
    [DataRow(9, 9, 0, 8, 13, 0)] // normalised, not moved
    [DataRow(8, 13, 1, 8, 13, 0)]
    [DataRow(0, 13, 1, 4, 8, 1)] // a long range moves from its start, as one unit
    [DataRow(13, 13, -1, 4, 8, -1)]
    public void MovingGoesWholeUnitsAndSaysHowFarItGot(int start, int end, int count, int from, int to, int moved)
    {
        var text = new ScreenText("one two three");

        TextRange range = text.Move(new TextRange(start, end), TextUnit.Word, count, out int actual);

        Assert.AreEqual((new TextRange(from, to), moved), (range, actual));
    }

    [TestMethod]
    public void MovingByLineCharacterAndDocument()
    {
        var lines = new ScreenText("a\nb\nc");
        Assert.AreEqual(new TextRange(4, 5), lines.Move(new TextRange(0, 0), TextUnit.Line, 2, out int moved));
        Assert.AreEqual(2, moved);
        Assert.AreEqual(new TextRange(0, 2), lines.Move(new TextRange(4, 5), TextUnit.Paragraph, -9, out moved));
        Assert.AreEqual(-2, moved);

        var marks = new ScreenText("aéb");
        Assert.AreEqual(new TextRange(3, 4), marks.Move(new TextRange(0, 0), TextUnit.Character, 2, out moved));
        Assert.AreEqual(2, moved);

        Assert.AreEqual(new TextRange(0, 5), new ScreenText("hello").Move(new TextRange(2, 2), TextUnit.Document, 3, out moved));
        Assert.AreEqual(0, moved);

        Assert.AreEqual(new TextRange(0, 0), new ScreenText("").Move(new TextRange(0, 0), TextUnit.Character, 4, out moved));
        Assert.AreEqual(0, moved);
        Assert.AreEqual(new TextRange(4, 5), lines.Move(new TextRange(0, 0), TextUnit.Character, int.MaxValue, out moved));
        Assert.AreEqual(4, moved);
    }

    [TestMethod]
    [DataRow(0, 4, TextEndpoint.End, 1, 0, 8, 1)]
    [DataRow(0, 4, TextEndpoint.Start, 1, 4, 4, 1)]
    [DataRow(0, 4, TextEndpoint.Start, 2, 8, 8, 2)] // the end is dragged along
    [DataRow(4, 8, TextEndpoint.End, -2, 0, 0, -2)] // and the start
    [DataRow(5, 8, TextEndpoint.Start, -1, 4, 8, -1)] // from inside a word back to its start
    [DataRow(4, 8, TextEndpoint.End, 100, 4, 13, 1)]
    [DataRow(0, 4, TextEndpoint.Start, -5, 0, 4, 0)]
    [DataRow(2, 6, TextEndpoint.End, 0, 2, 6, 0)]
    [DataRow(6, 6, TextEndpoint.End, 1, 6, 8, 1)] // an empty range grows
    public void EndpointsMoveAlone(int start, int end, TextEndpoint endpoint, int count, int from, int to, int moved)
    {
        var text = new ScreenText("one two three");

        TextRange range = text.MoveEndpointByUnit(new TextRange(start, end), endpoint, TextUnit.Word, count, out int actual);

        Assert.AreEqual((new TextRange(from, to), moved), (range, actual));
    }

    [TestMethod]
    public void EndpointsMoveByCharacterOverClusters()
    {
        var text = new ScreenText("aéb");

        Assert.AreEqual(new TextRange(0, 3), text.MoveEndpointByUnit(new TextRange(0, 1), TextEndpoint.End, TextUnit.Character, 1, out int moved));
        Assert.AreEqual(1, moved);
        Assert.AreEqual(new TextRange(1, 4), text.MoveEndpointByUnit(new TextRange(3, 4), TextEndpoint.Start, TextUnit.Character, -1, out moved));
        Assert.AreEqual(-1, moved);
        Assert.AreEqual(new TextRange(3, 3), new ScreenText("abc").MoveEndpointByUnit(new TextRange(2, 2), TextEndpoint.Start, TextUnit.Document, 3, out moved));
        Assert.AreEqual(1, moved);
    }

    [TestMethod]
    public void ARangeOutsideTheTextIsClamped()
    {
        var text = new ScreenText("abc");

        Assert.AreEqual(new TextRange(0, 3), text.ExpandToEnclosingUnit(new TextRange(-9, 99), TextUnit.Line));
        Assert.AreEqual(new TextRange(1, 2), text.ExpandToEnclosingUnit(new TextRange(1, 1), TextUnit.Character));
        Assert.AreEqual(new TextRange(2, 3), text.ExpandToEnclosingUnit(new TextRange(99, 99), TextUnit.Character));
        Assert.AreEqual(new TextRange(3, 3), text.MoveEndpointByUnit(new TextRange(9, 2), TextEndpoint.End, TextUnit.Document, 1, out int moved));
        Assert.AreEqual(0, moved);
    }

    [TestMethod]
    public void ANonexistentUnitAndNullArgumentsAreRefused()
    {
        var text = new ScreenText("abc");

        Assert.ThrowsExactly<ArgumentOutOfRangeException>(() => text.ExpandToEnclosingUnit(default, (TextUnit)42));
        Assert.ThrowsExactly<ArgumentOutOfRangeException>(() => text.Move(default, (TextUnit)(-1), 1, out _));
        Assert.ThrowsExactly<ArgumentNullException>(() => new ScreenText(null!));
        Assert.ThrowsExactly<ArgumentNullException>(() => text.Columns(0, null!));
        Assert.ThrowsExactly<ArgumentNullException>(() => text.LineSpans(default, null!));
        Assert.ThrowsExactly<ArgumentNullException>(() => ScreenText.Read(null!));
        Assert.ThrowsExactly<ArgumentNullException>(() => TextChange.Between(text, null!));
    }

    [TestMethod]
    public void TheUnitValuesAreUiAutomations()
    {
        TextUnit[] units = [TextUnit.Character, TextUnit.Format, TextUnit.Word, TextUnit.Line, TextUnit.Paragraph, TextUnit.Page, TextUnit.Document];
        for (int i = 0; i < units.Length; i++)
        {
            Assert.AreEqual(i, (int)units[i]);
        }
        Assert.AreEqual((0, 1), ((int)TextEndpoint.Start, (int)TextEndpoint.End));
    }

    [TestMethod]
    [DataRow("a\nb\nc", "a\nb\nc", -1, -1)]
    [DataRow("a\nb\nc", "a\nX\nc", 1, 1)]
    [DataRow("a\nb", "a\nb\nc", 2, 2)] // a line added
    [DataRow("a\nb\nc", "a\nc", 1, 1)] // a line removed
    [DataRow("a\nb\nc", "x\ny\nz", 0, 2)]
    [DataRow("a\nb\nc\nd", "a\nB\nC\nd", 1, 2)]
    [DataRow("a\nb", "a", 0, 0)]
    [DataRow("a", "", 0, 0)]
    [DataRow("", "a", 0, 0)]
    public void ChangesAreTheLinesThatDiffer(string before, string after, int first, int last)
    {
        TextChange? change = TextChange.Between(new ScreenText(before), new ScreenText(after));

        if (first < 0)
        {
            Assert.IsNull(change);
        }
        else
        {
            Assert.AreEqual(new TextChange(first, last), change);
        }
    }

    [TestMethod]
    public void WithoutAPreviousTextEveryLineChanged() =>
        Assert.AreEqual(new TextChange(0, 2), TextChange.Between(null, new ScreenText("a\nb\nc")));

    [TestMethod]
    public void ARealTerminalsTextCursorAndWideCellsLineUp()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("hi 世界\r\nx"u8);
        Assert.IsTrue(terminal.Update());
        ScreenText text = ScreenText.Read(terminal);
        // The view is a ref struct, so what the width callback needs is copied out of it.
        FrameView view = terminal.View;
        Cursor cursor = view.Cursor;
        int cols = view.Cols;
        byte[] widths = [.. view.Cells.ToArray().Select(c => c.Width)];
        Func<int, int> RowWidth(int row) => col => widths[(row * cols) + col];

        Assert.AreEqual("hi 世界\nx\n\n", text.Text, "a row each, trailing blanks of a row trimmed");
        Assert.AreEqual(4, text.LineCount);
        Assert.AreEqual(7, text.OffsetOf(cursor.Row, cursor.Col, RowWidth(cursor.Row)), "the cursor after x");
        Assert.AreEqual(3, text.OffsetOf(0, 3, RowWidth(0)));
        Assert.AreEqual(4, text.OffsetOf(0, 5, RowWidth(0)), "the column of 界 is 5, after the two of 世");
        CollectionAssert.AreEqual(
            new[] { new LineSpan(0, 3, 7) },
            text.LineSpans(new TextRange(3, 5), RowWidth(0)).ToArray());

        terminal.Feed("\r\nmore"u8);
        terminal.Update();
        TextChange? change = TextChange.Between(text, ScreenText.Read(terminal));
        Assert.AreEqual(new TextChange(2, 2), change, "only the third row gained text");
    }
}
