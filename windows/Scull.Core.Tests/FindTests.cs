namespace Scull.Core.Tests;

/// <summary>The core's search through <see cref="Terminal"/>, and the find bar's state as FindTests.swift checks it.</summary>
[TestClass]
public sealed class FindTests
{
    // Eight lines on a three-row screen: "ab" on lines 0, 2 and 4, the last two in history.
    private static Terminal History()
    {
        var terminal = new Terminal(10, 3);
        terminal.Feed("ab\r\nx\r\nab\r\nx\r\nab\r\nx\r\nx\r\nx"u8);
        terminal.Update();
        return terminal;
    }

    private static ulong Step(Terminal terminal, bool forward)
    {
        Assert.IsTrue(terminal.SearchStep(forward, out SearchMatch found));
        Assert.AreEqual((0, 1), (found.StartCol, found.EndCol));
        Assert.AreEqual(found.StartLine, found.EndLine);
        return found.StartLine;
    }

    [TestMethod]
    public void StepsGoBothWaysAndWrapAtTheEnds()
    {
        using Terminal terminal = History();
        Assert.IsTrue(terminal.SearchSet("ab", ignoreCase: false));
        Assert.AreEqual(3, terminal.SearchCount());

        Assert.AreEqual(4UL, Step(terminal, forward: false), "back from the bottom of the screen");
        Assert.AreEqual(2UL, Step(terminal, forward: false));
        Assert.AreEqual(4UL, Step(terminal, forward: true));
        Assert.AreEqual(0UL, Step(terminal, forward: true), "past the last match to the first");
        Assert.AreEqual(4UL, Step(terminal, forward: false), "before the first match to the last");
    }

    [TestMethod]
    public void AStepScrollsTheMatchIntoViewAsTheCurrentOne()
    {
        using Terminal terminal = History();
        terminal.SearchSet("ab", ignoreCase: false);
        terminal.SearchStep(forward: true, out _);
        terminal.SearchStep(forward: true, out _);
        terminal.Update();

        ReadOnlySpan<Cell> cells = terminal.View.Cells;
        int at = -1;
        for (int i = 0; i < cells.Length; i++)
        {
            if (cells[i].IsCurrentMatch)
            {
                at = i;
                break;
            }
        }
        Assert.IsGreaterThanOrEqualTo(0, at, "the current match is in the viewport");
        Assert.AreEqual('a', (char)cells[at].CodePoint);
        Assert.IsTrue(cells[at].IsMatch, "a current match is a match too");
        Assert.IsTrue(cells[at + 1].IsCurrentMatch);
        Assert.IsFalse(cells[at + 2].IsMatch);
    }

    [TestMethod]
    public void NoMatchOrNoSearchStepsNowhere()
    {
        using Terminal terminal = History();
        Assert.IsFalse(terminal.SearchStep(forward: true, out _), "no search yet");
        Assert.IsTrue(terminal.SearchSet("zz", ignoreCase: true));
        Assert.IsFalse(terminal.SearchStep(forward: true, out _));
        Assert.AreEqual(0, terminal.SearchCount());
    }

    [TestMethod]
    public void CaseFoldsOnlyWhenAsked()
    {
        using Terminal terminal = History();
        terminal.SearchSet("AB", ignoreCase: true);
        Assert.AreEqual(3, terminal.SearchCount());
        terminal.SearchSet("AB", ignoreCase: false);
        Assert.AreEqual(0, terminal.SearchCount());
    }

    [TestMethod]
    public void AnEmptyOrRefusedPatternEndsTheSearch()
    {
        using Terminal terminal = History();
        terminal.SearchSet("ab", ignoreCase: false);
        Assert.IsTrue(terminal.SearchSet("", ignoreCase: false));
        Assert.IsFalse(terminal.SearchStep(forward: true, out _));

        terminal.SearchSet("ab", ignoreCase: false);
        Assert.IsFalse(terminal.SearchSet(new string('a', Terminal.MaxSearchPattern + 1), ignoreCase: false), "too long");
        Assert.AreEqual(0, terminal.SearchCount());
        Assert.IsTrue(terminal.SearchSet(new string('a', Terminal.MaxSearchPattern), ignoreCase: false), "the longest taken");
    }

    [TestMethod]
    public void TheStateSearchesAtOnceAndStepsOnceSettled()
    {
        var find = new FindState();
        Assert.IsEmpty(find.Edit("x"), "a closed bar searches nothing");
        Assert.IsEmpty(find.Open());
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Search("fo", true) }, find.Edit("fo").ToArray());
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Search("foo", true) }, find.Edit("foo").ToArray());
        Assert.IsTrue(find.Unsettled);
        Assert.AreEqual("", find.Label);
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Move(true), new FindEffect.Count() }, find.Settle().ToArray());
        Assert.IsEmpty(find.Settle(), "settled once");
        find.Count = 3;
        Assert.AreEqual("3 matches", find.Label);
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Move(true) }, find.Step(forward: true).ToArray());
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Move(false) }, find.Step(forward: false).ToArray());
        // Toggling case is a new search, counted again.
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Search("foo", false) }, find.ToggleCase().ToArray());
        Assert.IsNull(find.Count);
        // A step before the pause stands in for the settle.
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Move(false), new FindEffect.Count() }, find.Step(forward: false).ToArray());
        Assert.IsEmpty(find.Settle());
    }

    [TestMethod]
    public void TheStateClosesAndReopensOnTheLastPattern()
    {
        var find = new FindState();
        find.Open();
        find.Edit("");
        Assert.IsFalse(find.CanStep);
        Assert.IsEmpty(find.Step(forward: true));
        Assert.IsEmpty(find.Settle());
        find.Edit("bar");
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Search("", true) }, find.Close().ToArray());
        Assert.IsFalse(find.IsOpen);
        Assert.IsFalse(find.CanStep);
        Assert.IsEmpty(find.Close());
        CollectionAssert.AreEqual(new FindEffect[] { new FindEffect.Search("bar", true) }, find.Open().ToArray());
        Assert.IsTrue(find.Unsettled);
    }

    [TestMethod]
    [DataRow(0, "No matches")]
    [DataRow(1, "1 match")]
    [DataRow(12, "12 matches")]
    [DataRow(Terminal.MaxSearchMatches, "10000+ matches")]
    public void TheLabelCountsMatches(int count, string label) =>
        Assert.AreEqual(label, new FindState { Count = count }.Label);
}
