namespace Scull.Core.Tests;

[TestClass]
public sealed class TextSnapshotTests
{
    private static readonly CellGeometry Geometry = new(10, 20, 100, 50);

    // 4 columns: "世" takes two cells and a spacer.
    private static TextSnapshot Wide(string text = "a世b\nxy") =>
        new(new ScreenText(text), Geometry, 4, [1, 2, 0, 1, 1, 1, 1, 1], cursorRow: 1, cursorCol: 1);

    [TestMethod]
    public void TheCaretIsWhereTheCursorIs()
    {
        TextSnapshot snapshot = Wide();

        Assert.AreEqual(new TextRange(5, 5), snapshot.Caret, "on y");
        Assert.AreEqual(new TextRange(0, 6), snapshot.Document);
    }

    [TestMethod]
    public void RectanglesAreInPixelsFromThePanesOrigin()
    {
        TextSnapshot snapshot = Wide();

        // 世 starts at column 1 and takes two cells.
        CollectionAssert.AreEqual(new[] { new PixelRect(110, 50, 20, 20) }, snapshot.Rectangles(new TextRange(1, 2)).ToArray());
        CollectionAssert.AreEqual(
            new[] { new PixelRect(100, 50, 40, 20), new PixelRect(100, 70, 20, 20) },
            snapshot.Rectangles(new TextRange(0, 6)).ToArray(),
            "one rectangle per row");
        CollectionAssert.AreEqual(new[] { new PixelRect(110, 70, 10, 20) }, snapshot.Rectangles(snapshot.Caret).ToArray(), "the caret is one cell");
    }

    [TestMethod]
    [DataRow(100, 50, 0, 1)] // a
    [DataRow(115, 55, 1, 2)] // the first cell of 世
    [DataRow(125, 55, 1, 2)] // its second cell
    [DataRow(135, 55, 2, 3)] // b
    [DataRow(105, 75, 4, 5)] // x on the second row
    [DataRow(-40, -40, 0, 1)] // above and left clamp to the first cell
    [DataRow(500, 75, 6, 6)] // past the end of the text
    [DataRow(105, 900, 4, 5)] // below clamps to the last row
    public void APointFindsTheCharacterUnderIt(double x, double y, int start, int end)
    {
        TextSnapshot snapshot = Wide("a世b\nxy");

        TextRange range = snapshot.RangeAt(x, y);

        Assert.AreEqual(new TextRange(start, end), range);
    }

    [TestMethod]
    public void APointPastTheTextIsAnEmptyRangeAtItsEnd()
    {
        var snapshot = new TextSnapshot(new ScreenText("ab"), Geometry, 4, [1, 1, 1, 1], 0, 0);

        Assert.AreEqual(new TextRange(2, 2), snapshot.RangeAt(300, 50));
        Assert.AreEqual(new TextRange(2, 2), new TextSnapshot(new ScreenText("ab"), default, 4, [], 0, 0).RangeAt(1, 1), "no cell size, no hit test");
    }

    [TestMethod]
    public void MissingCellsAreOneWide()
    {
        var snapshot = new TextSnapshot(new ScreenText("ab\ncd"), Geometry, 2, [], 5, 9);

        Assert.AreEqual(1, snapshot.WidthOf(0)(0));
        Assert.AreEqual(1, snapshot.WidthOf(9)(3));
        Assert.AreEqual(1, snapshot.WidthOf(-2)(-1));
        Assert.AreEqual(new TextRange(5, 5), snapshot.Caret, "a cursor past the text sits at its end");
    }

    [TestMethod]
    public void ARealTerminalIsCapturedWithItsCellsAndCursor()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("hi 世界\r\nx"u8);
        Assert.IsTrue(terminal.Update());

        TextSnapshot snapshot = TextSnapshot.Capture(terminal, Geometry);
        terminal.Feed("\r\nmore"u8);
        terminal.Update();

        Assert.AreEqual("hi 世界\nx\n\n", snapshot.Text.Text, "a later update does not change it");
        Assert.AreEqual(new TextRange(7, 7), snapshot.Caret);
        CollectionAssert.AreEqual(new[] { new PixelRect(130, 50, 40, 20) }, snapshot.Rectangles(new TextRange(3, 5)).ToArray());
        Assert.AreEqual(new TextRange(4, 5), snapshot.RangeAt(165, 55), "the second cell of 界");
        Assert.AreEqual(new TextRange(3, 4), snapshot.RangeAt(131, 55));
        Assert.ThrowsExactly<ArgumentNullException>(() => TextSnapshot.Capture(null!, Geometry));
    }

    [TestMethod]
    public void ArgumentsAreChecked()
    {
        Assert.ThrowsExactly<ArgumentNullException>(() => new TextSnapshot(null!, Geometry, 1, [], 0, 0));
        Assert.ThrowsExactly<ArgumentNullException>(() => new TextSnapshot(new ScreenText(""), Geometry, 1, null!, 0, 0));
    }

    [TestMethod]
    public void TextIsFoundForwardBackwardAndInsideARange()
    {
        var text = new ScreenText("one two one TWO");

        Assert.AreEqual(new TextRange(0, 3), text.Find(text.Document, "one", backward: false, ignoreCase: false));
        Assert.AreEqual(new TextRange(8, 11), text.Find(text.Document, "one", backward: true, ignoreCase: false));
        Assert.AreEqual(new TextRange(8, 11), text.Find(new TextRange(1, 15), "one", backward: false, ignoreCase: false));
        Assert.AreEqual(new TextRange(4, 7), text.Find(text.Document, "TWO", backward: false, ignoreCase: true));
        Assert.AreEqual(new TextRange(12, 15), text.Find(text.Document, "TWO", backward: false, ignoreCase: false));
        Assert.AreEqual(new TextRange(12, 15), text.Find(text.Document, "two", backward: true, ignoreCase: true));
        Assert.IsNull(text.Find(text.Document, "three", backward: false, ignoreCase: true));
        Assert.IsNull(text.Find(new TextRange(0, 2), "one", backward: false, ignoreCase: false), "a match must lie inside the range");
        Assert.IsNull(text.Find(new TextRange(0, 2), "one", backward: true, ignoreCase: false));
        Assert.IsNull(text.Find(text.Document, "", backward: false, ignoreCase: false));
        Assert.IsNull(new ScreenText("").Find(default, "a", backward: true, ignoreCase: false));
        Assert.ThrowsExactly<ArgumentNullException>(() => text.Find(text.Document, null!, false, false));
    }

    [TestMethod]
    public void EndpointsCompareAndDragEachOther()
    {
        var range = new TextRange(2, 6);
        var other = new TextRange(4, 9);

        Assert.IsTrue(range.CompareEndpoints(TextEndpoint.Start, other, TextEndpoint.Start) < 0);
        Assert.IsTrue(range.CompareEndpoints(TextEndpoint.End, other, TextEndpoint.Start) > 0);
        Assert.AreEqual(0, range.CompareEndpoints(TextEndpoint.End, new TextRange(6, 6), TextEndpoint.Start));
        Assert.AreEqual(new TextRange(4, 6), range.WithEndpoint(TextEndpoint.Start, other.At(TextEndpoint.Start)));
        Assert.AreEqual(new TextRange(9, 9), range.WithEndpoint(TextEndpoint.Start, other.At(TextEndpoint.End)), "the end follows a start that passes it");
        Assert.AreEqual(new TextRange(1, 1), range.WithEndpoint(TextEndpoint.End, 1), "and the start an end that passes it");
        Assert.AreEqual(new TextRange(2, 9), range.WithEndpoint(TextEndpoint.End, 9));
        Assert.IsTrue(new TextRange(3, 3).IsEmpty);
        Assert.AreEqual(4, range.Length);
    }

    [TestMethod]
    public void NothingIsReadWhileNobodyListens()
    {
        var tracker = new TextChangeTracker();
        int reads = 0;
        ScreenText Read(string text)
        {
            reads++;
            return new ScreenText(text);
        }

        Assert.IsNull(tracker.Observe(listening: false, () => Read("a")));
        Assert.AreEqual(0, reads);

        Assert.AreEqual(new TextChange(0, 0), tracker.Observe(listening: true, () => Read("a")), "the client has seen nothing yet");
        Assert.IsNull(tracker.Observe(listening: true, () => Read("a")), "a redraw of the same text is not news");
        Assert.AreEqual(new TextChange(1, 1), tracker.Observe(listening: true, () => Read("a\nb")));
        Assert.AreEqual(3, reads);

        Assert.IsNull(tracker.Observe(listening: false, () => Read("never")));
        Assert.AreEqual(new TextChange(0, 1), tracker.Observe(listening: true, () => Read("a\nb")), "listening again starts over");
        Assert.ThrowsExactly<ArgumentNullException>(() => tracker.Observe(true, null!));
    }
}
