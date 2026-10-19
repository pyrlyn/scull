using Scull.Native;

namespace Scull.Core.Tests;

// The first three tests are the Swift ones in macos/Tests/ScullKitTests/PaneTests.swift.
[TestClass]
public sealed class PaneTests
{
    // Built with `--features test-hooks` (`just windows-core-test`).
    [System.Runtime.InteropServices.DllImport("scull_ffi", EntryPoint = "tt_term_test_panic")]
    private static extern tt_status TestPanic(nint term);

    private static readonly int[] OnlyFirst = [0];

    [TestMethod]
    public void SplittingFocusesTheNewPaneAfterTheOldOne()
    {
        var tree = new PaneTree();
        CollectionAssert.AreEqual(OnlyFirst, tree.Panes.ToArray());
        Assert.AreEqual(0, tree.Focused);

        int right = tree.SplitFocused(SplitAxis.SideBySide);
        CollectionAssert.AreEqual(new[] { 0, right }, tree.Panes.ToArray());
        Assert.AreEqual(right, tree.Focused);

        // Splitting the first pane again nests inside its share only.
        tree.Focus(0);
        int below = tree.SplitFocused(SplitAxis.Stacked);
        CollectionAssert.AreEqual(new[] { 0, below, right }, tree.Panes.ToArray());
        Assert.AreEqual(
            new PaneNode.Split(
                SplitAxis.SideBySide,
                new PaneNode.Split(SplitAxis.Stacked, new PaneNode.Leaf(0), new PaneNode.Leaf(below)),
                new PaneNode.Leaf(right)),
            tree.Root);
    }

    [TestMethod]
    public void ClosingGivesTheShareToTheSiblingAndMovesFocus()
    {
        var tree = new PaneTree();
        int b = tree.SplitFocused(SplitAxis.SideBySide);
        int c = tree.SplitFocused(SplitAxis.Stacked);

        Assert.IsTrue(tree.Close(c));
        Assert.AreEqual(
            new PaneNode.Split(SplitAxis.SideBySide, new PaneNode.Leaf(0), new PaneNode.Leaf(b)),
            tree.Root);
        Assert.AreEqual(b, tree.Focused, "the pane before takes it when the last one closes");

        tree.Focus(0);
        Assert.IsTrue(tree.Close(0));
        Assert.AreEqual(new PaneNode.Leaf(b), tree.Root);
        Assert.AreEqual(b, tree.Focused);

        Assert.IsFalse(tree.Close(b), "the last pane is the window's to close");
        Assert.IsFalse(tree.Close(99));
    }

    [TestMethod]
    public void FocusWalksTheReadingOrderAndWraps()
    {
        var tree = new PaneTree();
        int b = tree.SplitFocused(SplitAxis.SideBySide);
        int c = tree.SplitFocused(SplitAxis.Stacked);

        tree.FocusNext();
        Assert.AreEqual(0, tree.Focused);
        tree.FocusNext(-1);
        Assert.AreEqual(c, tree.Focused);
        tree.Focus(b);
        tree.FocusNext();
        Assert.AreEqual(c, tree.Focused);
        tree.Focus(99);
        Assert.AreEqual(c, tree.Focused);
    }

    [TestMethod]
    public void IdsAreNeverReused()
    {
        var tree = new PaneTree();
        int b = tree.SplitFocused(SplitAxis.SideBySide);
        tree.Close(b);
        Assert.AreEqual(b + 1, tree.NextId);
        Assert.AreEqual(b + 1, tree.SplitFocused(SplitAxis.Stacked));
    }

    [TestMethod]
    public void LayoutCutsTheSurfaceIntoViewportsInReadingOrder()
    {
        var tree = new PaneTree();
        Assert.AreEqual(new PaneViewport(0, 0, 0, 801, 600), tree.Layout(801, 600).Single());

        int right = tree.SplitFocused(SplitAxis.SideBySide);
        tree.Focus(0);
        int below = tree.SplitFocused(SplitAxis.Stacked);

        // The odd pixel goes to the second pane, and a divider takes its own column or row.
        PaneViewport[] plain = [.. tree.Layout(801, 601)];
        CollectionAssert.AreEqual(
            new[]
            {
                new PaneViewport(0, 0, 0, 400, 300),
                new PaneViewport(below, 0, 300, 400, 301),
                new PaneViewport(right, 400, 0, 401, 601),
            },
            plain);

        PaneViewport[] divided = [.. tree.Layout(801, 601, divider: 1)];
        CollectionAssert.AreEqual(
            new[]
            {
                new PaneViewport(0, 0, 0, 400, 300),
                new PaneViewport(below, 0, 301, 400, 300),
                new PaneViewport(right, 401, 0, 400, 601),
            },
            divided);
    }

    [TestMethod]
    public void LayoutNeverOverlapsOrLeavesTheSurface()
    {
        var tree = new PaneTree();
        for (int i = 0; i < 6; i++)
        {
            tree.SplitFocused(i % 2 == 0 ? SplitAxis.SideBySide : SplitAxis.Stacked);
            tree.FocusNext(-1 - (i % 3));
        }

        foreach ((int width, int height) in new[] { (1000, 700), (5, 3), (0, 0) })
        {
            PaneViewport[] views = [.. tree.Layout(width, height, divider: 2)];
            Assert.AreEqual(tree.Panes.Count, views.Length);
            foreach (PaneViewport v in views)
            {
                Assert.IsTrue(v.Width >= 0 && v.Height >= 0 && v.X >= 0 && v.Y >= 0);
                Assert.IsTrue(v.X + v.Width <= width && v.Y + v.Height <= height, $"{v} leaves {width}x{height}");
            }

            for (int i = 0; i < views.Length; i++)
            {
                for (int j = i + 1; j < views.Length; j++)
                {
                    Assert.IsFalse(Overlap(views[i], views[j]), $"{views[i]} overlaps {views[j]}");
                }
            }
        }
    }

    [TestMethod]
    public void ANegativeSurfaceIsRejected()
    {
        var tree = new PaneTree();
        Assert.ThrowsExactly<ArgumentOutOfRangeException>(() => tree.Layout(-1, 10));
        Assert.ThrowsExactly<ArgumentOutOfRangeException>(() => tree.Layout(10, 10, -1));
    }

    [TestMethod]
    public void APaneKeepsItsTerminalThroughItsNeighboursSplitsAndCloses()
    {
        var made = new List<int>();
        using var panes = new PaneSet(id =>
        {
            made.Add(id);
            return new Terminal(10, 3);
        });
        Terminal first = panes[0];
        first.Feed("keep"u8);

        int b = panes.Split(SplitAxis.SideBySide);
        int c = panes.Split(SplitAxis.Stacked);
        Assert.IsTrue(panes.Close(b));
        Assert.IsFalse(panes.Close(b), "already closed");
        Assert.AreSame(first, panes[0]);
        Assert.AreEqual("keep", first.ReadText(0, 1));
        CollectionAssert.AreEqual(new[] { 0, b, c }, made);
        Assert.IsFalse(panes.Close(99));
        Assert.IsTrue(panes.Close(0) && !panes.Close(c), "the last pane stays");
    }

    [TestMethod]
    public void AFailedTerminalLeavesTheTreeAsItWas()
    {
        int calls = 0;
        using var panes = new PaneSet(_ => ++calls > 1 ? throw new ScullException(Status.Invalid, "spawn") : new Terminal(10, 3));
        Assert.ThrowsExactly<ScullException>(() => panes.Split(SplitAxis.SideBySide));
        CollectionAssert.AreEqual(OnlyFirst, panes.Tree.Panes.ToArray());
    }

    [TestMethod]
    public void APoisonedTerminalPoisonsOnePaneAndSiblingsKeepAnswering()
    {
        using var panes = new PaneSet(_ => new Terminal(10, 3));
        int sick = panes.Split(SplitAxis.SideBySide);
        int well = panes.Split(SplitAxis.Stacked);

        using (Lease lease = panes[sick].LeaseTerm())
        {
            Assert.AreEqual(tt_status.TT_PANIC, TestPanic(lease.Pointer));
        }

        Assert.ThrowsExactly<TerminalPoisonedException>(() => panes[sick].Feed("x"u8));
        CollectionAssert.AreEqual(new[] { sick }, panes.Poisoned.ToArray());

        foreach (int pane in new[] { 0, well })
        {
            panes[pane].Feed("ok"u8);
            Assert.IsTrue(panes[pane].Update());
            Assert.AreEqual("ok", panes[pane].ReadText(0, 1));
            Assert.IsFalse(panes[pane].IsPoisoned);
        }

        Assert.IsTrue(panes.Close(sick), "a poisoned pane can still be closed");
        Assert.AreEqual(0, panes.Poisoned.Count());
    }

    [TestMethod]
    public void APixelBelongsToTheViewportThatContainsItAndNoneOverADivider()
    {
        var tree = new PaneTree();
        int right = tree.SplitFocused(SplitAxis.SideBySide);
        PaneViewport[] views = [.. tree.Layout(101, 50, divider: 1)];

        Assert.AreEqual(0, PaneTree.PaneAt(views, 0, 0));
        Assert.AreEqual(0, PaneTree.PaneAt(views, 49.9, 49.9));
        Assert.IsNull(PaneTree.PaneAt(views, 50, 10), "the divider column");
        Assert.AreEqual(right, PaneTree.PaneAt(views, 51, 10));
        Assert.IsNull(PaneTree.PaneAt(views, 101, 10), "right of the surface");
        Assert.IsNull(PaneTree.PaneAt(views, 10, -1), "above it");
    }

    [TestMethod]
    [DataRow(0x54, PaneCommand.NewTab)]
    [DataRow(0x46, PaneCommand.Find)]
    [DataRow(0x4E, PaneCommand.NewWindow)]
    [DataRow(0x44, PaneCommand.SplitRight)]
    [DataRow(0x45, PaneCommand.SplitDown)]
    [DataRow(0x57, PaneCommand.ClosePane)]
    [DataRow(0x43, PaneCommand.Copy)]
    [DataRow(0xDD, PaneCommand.NextPane)]
    [DataRow(0xDB, PaneCommand.PreviousPane)]
    [DataRow(0x26, PaneCommand.PreviousPrompt)]
    [DataRow(0x28, PaneCommand.NextPrompt)]
    public void CtrlShiftChordsRunPaneCommands(int key, PaneCommand command)
    {
        Assert.AreEqual(command, PaneCommands.Match(key, KeyModifiers.Control | KeyModifiers.Shift));
        // Lock keys do not stop a chord.
        Assert.AreEqual(command, PaneCommands.Match(key, KeyModifiers.Control | KeyModifiers.Shift | KeyModifiers.CapsLock | KeyModifiers.NumLock));
    }

    [TestMethod]
    public void CtrlCommaOpensSettingsAndNothingElseDoes()
    {
        Assert.AreEqual(PaneCommand.OpenSettings, PaneCommands.Match(0xBC, KeyModifiers.Control));
        Assert.AreEqual(PaneCommand.OpenSettings, PaneCommands.Match(0xBC, KeyModifiers.Control | KeyModifiers.NumLock));
        Assert.IsNull(PaneCommands.Match(0xBC, KeyModifiers.None));
        Assert.IsNull(PaneCommands.Match(0xBC, KeyModifiers.Control | KeyModifiers.Shift));
        Assert.IsNull(PaneCommands.Match(0xBC, KeyModifiers.Control | KeyModifiers.Alt), "AltGr+comma types a character");
    }

    [TestMethod]
    [DataRow(0x54, KeyModifiers.Control)]
    [DataRow(0x54, KeyModifiers.Shift)]
    [DataRow(0x54, KeyModifiers.None)]
    [DataRow(0x54, KeyModifiers.Control | KeyModifiers.Shift | KeyModifiers.Alt)]
    [DataRow(0x56, KeyModifiers.Control | KeyModifiers.Shift)] // paste stays paste
    [DataRow(0x26, KeyModifiers.Control)] // Ctrl+Up is the program's
    [DataRow(0x26, KeyModifiers.None)]
    [DataRow(0x28, KeyModifiers.Shift)]
    public void OtherKeysReachTheProgram(int key, KeyModifiers held) => Assert.IsNull(PaneCommands.Match(key, held));

    private static bool Overlap(PaneViewport a, PaneViewport b) =>
        a.Width > 0 && a.Height > 0 && b.Width > 0 && b.Height > 0
        && a.X < b.X + b.Width && b.X < a.X + a.Width
        && a.Y < b.Y + b.Height && b.Y < a.Y + a.Height;

    [TestMethod]
    public void CtrlInsertCopiesLikeCtrlShiftCOnlyWithASelection()
    {
        const int Insert = 0x2D;
        Assert.AreEqual(PaneCommand.Copy, PaneCommands.Match(Insert, KeyModifiers.Control, extended: true, () => true));
        Assert.AreEqual(
            PaneCommand.Copy,
            PaneCommands.Match(Insert, KeyModifiers.Control | KeyModifiers.NumLock | KeyModifiers.CapsLock, extended: true, () => true),
            "the locks do not count");
        Assert.IsNull(PaneCommands.Match(Insert, KeyModifiers.Control, extended: true, () => false), "with no selection the program gets it");
        Assert.IsNull(PaneCommands.Match(Insert, KeyModifiers.Control, extended: false, () => true), "the keypad's 0 is not Insert");
        Assert.IsNull(PaneCommands.Match(Insert, KeyModifiers.Shift, extended: true, () => true), "Shift+Insert is paste");
        Assert.IsNull(PaneCommands.Match(Insert, KeyModifiers.Control | KeyModifiers.Alt, extended: true, () => true));
    }

    [TestMethod]
    public void TheSelectionIsAskedOnlyForCtrlInsert()
    {
        bool asked = false;
        bool Ask()
        {
            asked = true;
            return true;
        }

        Assert.AreEqual(PaneCommand.Copy, PaneCommands.Match(0x43, KeyModifiers.Control | KeyModifiers.Shift, extended: false, Ask));
        Assert.IsNull(PaneCommands.Match(0x56, KeyModifiers.Control, extended: false, Ask));
        Assert.IsFalse(asked);
        Assert.AreEqual(PaneCommand.Copy, PaneCommands.Match(0x2D, KeyModifiers.Control, extended: true, Ask));
        Assert.IsTrue(asked);
    }
}
