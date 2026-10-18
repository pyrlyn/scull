using System.Text;

namespace Scull.Core.Tests;

[TestClass]
public sealed class SelectionSnapshotTests
{
    private static readonly CellGeometry Geometry = new(10, 20, 0, 0);

    private static Terminal Fed(string text)
    {
        var terminal = new Terminal(20, 4);
        terminal.Feed(Encoding.UTF8.GetBytes(text));
        return terminal;
    }

    private static TextSnapshot Snapshot(Terminal terminal)
    {
        terminal.Update();
        return TextSnapshot.Capture(terminal, Geometry);
    }

    [TestMethod]
    public void NoSelectionIsNoRange()
    {
        using Terminal terminal = Fed("hello");
        Assert.IsNull(Snapshot(terminal).Selection);
    }

    [TestMethod]
    public void ASelectionIsTheRangeOfItsCells()
    {
        using Terminal terminal = Fed("hello world");
        terminal.Select(SelectionKind.Cell, 0, 1);
        terminal.ExtendSelection(0, 3);

        TextSnapshot snapshot = Snapshot(terminal);
        Assert.AreEqual(new TextRange(1, 4), snapshot.Selection);
        Assert.AreEqual("ell", snapshot.Text.GetText(snapshot.Selection!.Value));
        Assert.AreEqual(terminal.SelectionText(), snapshot.Text.GetText(snapshot.Selection.Value));
    }

    [TestMethod]
    public void ASelectionAcrossRowsReadsAsTheCoreCopiesIt()
    {
        using Terminal terminal = Fed("one\r\ntwo\r\nthree");
        terminal.Select(SelectionKind.Cell, 0, 1);
        terminal.ExtendSelection(2, 2);

        TextSnapshot snapshot = Snapshot(terminal);
        string text = snapshot.Text.GetText(snapshot.Selection!.Value);
        // UIA keeps the newline as "\n"; the core's copy ends rows the same way.
        Assert.AreEqual("ne\ntwo\nthr", text);
        Assert.AreEqual(terminal.SelectionText(), text);
    }

    [TestMethod]
    public void AWideCharacterIsSelectedWhole()
    {
        using Terminal terminal = Fed("a世界b");
        // Cells: a, 世, spacer, 界, spacer, b. Selecting the second half of 世 through 界 takes both.
        terminal.Select(SelectionKind.Cell, 0, 2);
        terminal.ExtendSelection(0, 3);

        TextSnapshot snapshot = Snapshot(terminal);
        Assert.AreEqual("世界", snapshot.Text.GetText(snapshot.Selection!.Value));
        Assert.AreEqual(terminal.SelectionText(), snapshot.Text.GetText(snapshot.Selection.Value));
    }

    [TestMethod]
    public void AWordSelectionIsTheWord()
    {
        using Terminal terminal = Fed("hello world");
        terminal.Select(SelectionKind.Word, 0, 8);

        TextSnapshot snapshot = Snapshot(terminal);
        Assert.AreEqual("world", snapshot.Text.GetText(snapshot.Selection!.Value));
    }

    [TestMethod]
    public void ASelectionPastTheTextEndsAtTheTextsEnd()
    {
        using Terminal terminal = Fed("hi");
        terminal.Select(SelectionKind.Cell, 0, 0);
        terminal.ExtendSelection(0, 8);

        TextSnapshot snapshot = Snapshot(terminal);
        Assert.AreEqual("hi", snapshot.Text.GetText(snapshot.Selection!.Value));
    }

    [TestMethod]
    public void ClearingTheSelectionRemovesIt()
    {
        using Terminal terminal = Fed("hello");
        terminal.Select(SelectionKind.Cell, 0, 0);
        terminal.ExtendSelection(0, 2);
        Assert.IsNotNull(Snapshot(terminal).Selection);

        terminal.ClearSelection();
        Assert.IsNull(Snapshot(terminal).Selection);
    }

    [TestMethod]
    public void CellsOfARangeSelectThatRangeInTheCore()
    {
        using Terminal terminal = Fed("a世界b\r\nxyz");
        TextSnapshot snapshot = Snapshot(terminal);

        // 世界 is offsets 1 to 3 of the text.
        CellSpan cells = snapshot.CellsOf(new TextRange(1, 3))!.Value;
        Assert.AreEqual(new CellSpan(0, 1, 0, 4), cells, "世 starts at column 1 and 界 ends at column 4");

        terminal.Select(SelectionKind.Cell, (ushort)cells.StartRow, (ushort)cells.StartCol);
        terminal.ExtendSelection((ushort)cells.EndRow, (ushort)cells.EndCol);
        Assert.AreEqual("世界", terminal.SelectionText());
        Assert.AreEqual(new TextRange(1, 3), Snapshot(terminal).Selection);
    }

    [TestMethod]
    public void CellsOfARangeSpanningRows()
    {
        using Terminal terminal = Fed("abc\r\ndef");
        TextSnapshot snapshot = Snapshot(terminal);

        // "bc\nde": the newline has no cell of its own.
        Assert.AreEqual(new CellSpan(0, 1, 1, 1), snapshot.CellsOf(new TextRange(1, 6)));
        Assert.AreEqual(new CellSpan(0, 1, 0, 2), snapshot.CellsOf(new TextRange(1, 4)), "a range ending in the newline stops at the row's last character");
    }

    [TestMethod]
    public void ARangeWithNoCharacterHasNoCells()
    {
        using Terminal terminal = Fed("abc\r\n\r\ndef");
        TextSnapshot snapshot = Snapshot(terminal);

        Assert.IsNull(snapshot.CellsOf(new TextRange(2, 2)));
        Assert.IsNull(snapshot.CellsOf(new TextRange(4, 5)), "an empty row's newline");
        Assert.IsNull(snapshot.CellsOf(new TextRange(100, 200)), "past the text");
    }
}

[TestClass]
public sealed class SelectionTrackerTests
{
    [TestMethod]
    public void NothingIsReadWhileNobodyListens()
    {
        var tracker = new SelectionTracker();
        Assert.IsFalse(tracker.Observe(false, () => throw new InvalidOperationException("read")));
    }

    [TestMethod]
    public void TheFirstLookIsABaselineAndLaterChangesAreReported()
    {
        var tracker = new SelectionTracker();
        Assert.IsFalse(tracker.Observe(true, () => new TextRange(2, 2)));
        Assert.IsFalse(tracker.Observe(true, () => new TextRange(2, 2)));
        Assert.IsTrue(tracker.Observe(true, () => new TextRange(2, 5)), "a selection began");
        Assert.IsFalse(tracker.Observe(true, () => new TextRange(2, 5)));
        Assert.IsTrue(tracker.Observe(true, () => new TextRange(6, 6)), "back to a caret, elsewhere");
    }

    [TestMethod]
    public void ListeningAgainStartsFromANewBaseline()
    {
        var tracker = new SelectionTracker();
        tracker.Observe(true, () => new TextRange(0, 0));
        tracker.Observe(false, () => new TextRange(0, 0));
        Assert.IsFalse(tracker.Observe(true, () => new TextRange(3, 3)));
    }
}
