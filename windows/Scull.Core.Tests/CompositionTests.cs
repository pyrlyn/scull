namespace Scull.Core.Tests;

[TestClass]
public sealed class CompositionTests
{
    [TestMethod]
    public void ThePreeditShowsAtTheCursorWithTheCaretInside()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("ab"u8);
        terminal.Update();
        Assert.AreEqual(0, terminal.View.Preedit.Cols, "none until the input method composes");

        terminal.SetPreedit("日本語", caret: 2);
        Assert.IsTrue(terminal.Update());

        FrameView view = terminal.View;
        Assert.AreEqual(new Preedit(0, 2, 6), view.Preedit, "three wide characters from the cursor");
        Assert.AreEqual(new Cursor(0, 6, true), view.Cursor, "the cursor moves to the caret, after two of them");
        Assert.IsTrue(view.Dirty.Contains((ushort)0));
        Assert.AreEqual('日', (char)view.Cells[2].CodePoint, "the text is in the frame's cells");
        Assert.AreEqual("ab", terminal.ReadText(0, 1), "and not on the screen");
    }

    [TestMethod]
    public void AnEmptyPreeditClearsIt()
    {
        using var terminal = new Terminal(20, 4);
        terminal.SetPreedit("ni", 2);
        terminal.Update();
        Assert.AreEqual(2, terminal.View.Preedit.Cols);

        terminal.SetPreedit("", 0);
        terminal.Update();

        Assert.AreEqual(0, terminal.View.Preedit.Cols);
        Assert.AreEqual(new Cursor(0, 0, true), terminal.View.Cursor);
    }

    [TestMethod]
    public void TheCaretIsCountedInUtf16AndClamped()
    {
        using var terminal = new Terminal(20, 4);

        terminal.SetPreedit("a😀b", caret: 3); // after the pair: 😀 is two UTF-16 units
        terminal.Update();
        Assert.AreEqual(3, terminal.View.Cursor.Col, "a, then the wide 😀");

        terminal.SetPreedit("ab", caret: 99);
        terminal.Update();
        Assert.AreEqual(2, terminal.View.Cursor.Col);

        terminal.SetPreedit("ab", caret: -4);
        terminal.Update();
        Assert.AreEqual(0, terminal.View.Cursor.Col);

        terminal.SetPreedit("a😀b", caret: 2); // inside the pair
        terminal.Update();
        Assert.AreEqual(1, terminal.View.Cursor.Col, "moved back onto the start of 😀");
        Assert.ThrowsExactly<ArgumentNullException>(() => terminal.SetPreedit(null!, 0));
    }

    [TestMethod]
    public void ACommittedTextGoesToTheChildNotIntoThePreedit()
    {
        using var terminal = new Terminal(20, 4);
        terminal.SetPreedit("かな", 2);
        terminal.Update();

        terminal.SetPreedit("", 0);
        terminal.Update();

        Assert.AreEqual("", terminal.ReadText(0, 1), "the composing text is never written to the screen");
    }

    [TestMethod]
    public void TheTrackerKnowsWhenATextIsReadyToRead()
    {
        var tracker = new CompositionTracker();
        Assert.IsFalse(tracker.Active);
        Assert.IsFalse(tracker.Ended(), "an end with no start is ignored");
        Assert.IsFalse(tracker.Pending);

        tracker.Started();
        tracker.Started();
        Assert.IsTrue(tracker.Active);
        Assert.IsFalse(tracker.Ended(), "one is still open");
        Assert.IsFalse(tracker.Pending);

        Assert.IsTrue(tracker.Ended());
        Assert.IsFalse(tracker.Active);
        Assert.IsTrue(tracker.Pending, "a key now must read the text first");

        tracker.Read();
        Assert.IsFalse(tracker.Pending);

        tracker.Started();
        tracker.Reset();
        Assert.IsFalse(tracker.Active);
    }

    [TestMethod]
    public void OnlyTextBeforeTheFirstCompositionIsFinished()
    {
        CompositionSplit split = CompositionSplit.Of([("今日", false), ("は", true), ("!", false), ("よ", true)], caret: 5);

        Assert.AreEqual("今日", split.Finished);
        Assert.AreEqual("は!よ", split.Active, "a run between two compositions is not final");
        Assert.AreEqual(3, split.Caret, "counted from the start of the active text");
    }

    [TestMethod]
    [DataRow(0, 0, "caret before the active text")]
    [DataRow(9, 2, "caret past its end")]
    public void TheCaretStaysInsideTheActiveText(int caret, int expected, string why)
    {
        CompositionSplit split = CompositionSplit.Of([("ab", false), ("cd", true)], caret);

        Assert.AreEqual(expected, split.Caret, why);
    }

    [TestMethod]
    public void NothingComposingMeansAllTextIsFinished()
    {
        CompositionSplit split = CompositionSplit.Of([("ab", false), ("c", false)], 3);

        Assert.AreEqual(new CompositionSplit("abc", "", 0), split);
        Assert.AreEqual(new CompositionSplit("", "", 0), CompositionSplit.Of([], 0));
        Assert.ThrowsExactly<ArgumentNullException>(() => CompositionSplit.Of(null!, 0));
    }

    [TestMethod]
    public void TheCandidateBoxIsTheCellsFromTheCursorInThePanesPixels()
    {
        var geometry = new CellGeometry(10, 20, 100, 50);

        Assert.AreEqual(new PixelRect(130, 90, 60, 20), geometry.Box(2, 3, 6));
        Assert.AreEqual(new PixelRect(100, 50, 10, 20), geometry.Box(0, 0, 0), "never narrower than a cell");
    }

    [TestMethod]
    public void OnlyVkProcesskeyIsTheInputMethodsKey()
    {
        Assert.IsTrue(WindowsKeys.IsProcessKey(0xE5));
        Assert.IsFalse(WindowsKeys.IsProcessKey(0x0D));
        Assert.IsFalse(WindowsKeys.IsProcessKey(0xE7), "VK_PACKET is typed text, not a held key");
    }
}
