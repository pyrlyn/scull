namespace Scull.Core.Tests;

// The wrappers against a real Terminal; the gesture logic needs none.
[TestClass]
public sealed class SelectionTests
{
    private static Terminal Fed()
    {
        var terminal = new Terminal(20, 4);
        terminal.Feed("hello world\r\nsecond line"u8);
        terminal.Update();
        return terminal;
    }

    [TestMethod]
    public void NothingIsSelectedUntilOneStarts()
    {
        using Terminal terminal = Fed();

        Assert.IsFalse(terminal.HasSelection);
        Assert.IsNull(terminal.SelectionText());
        // No selection is no change, and clearing none is fine.
        terminal.ExtendSelection(0, 3);
        terminal.ClearSelection();
        Assert.IsNull(terminal.SelectionText());
    }

    [TestMethod]
    [DataRow(SelectionKind.Word, 0, 7, "world")]
    [DataRow(SelectionKind.Line, 1, 3, "second line")]
    [DataRow(SelectionKind.Cell, 0, 4, "o")]
    public void AStartSelectsTheCellWordOrLineUnderIt(SelectionKind kind, int row, int col, string text)
    {
        using Terminal terminal = Fed();

        terminal.Select(kind, (ushort)row, (ushort)col);

        Assert.IsTrue(terminal.HasSelection);
        Assert.AreEqual(text, terminal.SelectionText());
    }

    [TestMethod]
    public void ExtendingMovesTheFreeEndAndAClearDropsIt()
    {
        using Terminal terminal = Fed();
        terminal.Select(SelectionKind.Cell, 0, 0);

        terminal.ExtendSelection(0, 4);
        Assert.AreEqual("hello", terminal.SelectionText());
        terminal.ExtendSelection(1, 5);
        Assert.AreEqual("hello world\nsecond", terminal.SelectionText());

        terminal.ClearSelection();
        Assert.IsFalse(terminal.HasSelection);
        Assert.IsNull(terminal.SelectionText());
    }

    [TestMethod]
    public void ABlockSelectsTheSameColumnsOfEveryRow()
    {
        using Terminal terminal = Fed();

        terminal.Select(SelectionKind.Block, 0, 0);
        terminal.ExtendSelection(1, 2);

        Assert.AreEqual("hel\nsec", terminal.SelectionText());
    }

    [TestMethod]
    public void ACellPastTheGridIsClamped()
    {
        using Terminal terminal = Fed();

        terminal.Select(SelectionKind.Line, 1, 3);
        terminal.ExtendSelection(ushort.MaxValue, ushort.MaxValue);

        // The free end lands on the last cell of the last row (3), two blank rows after the text.
        Assert.AreEqual("second line\n\n", terminal.SelectionText());
    }

    [TestMethod]
    public void TextLongerThanTheFirstGuessIsStillRead()
    {
        using var terminal = new Terminal(200, 50);
        for (int row = 0; row < 49; row++)
        {
            terminal.Feed(System.Text.Encoding.UTF8.GetBytes(new string('x', 190) + "\r\n"));
        }
        terminal.Update();

        terminal.Select(SelectionKind.Cell, 0, 0);
        terminal.ExtendSelection(48, 189);

        Assert.AreEqual(49 * 190 + 48, terminal.SelectionText()!.Length);
    }
}
