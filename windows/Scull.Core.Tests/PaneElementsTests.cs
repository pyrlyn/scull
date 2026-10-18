namespace Scull.Core.Tests;

[TestClass]
public sealed class PaneElementsTests
{
    [TestMethod]
    public void OnePaneIsOneFocusedElementOverTheWholeSurface()
    {
        var tree = new PaneTree();

        PaneElement only = PaneElements.Describe(tree, tree.Layout(800, 600)).Single();

        Assert.AreEqual(new PaneElement(0, 1, "Terminal, pane 1", new PixelRect(0, 0, 800, 600), true), only);
    }

    [TestMethod]
    public void PanesComeInReadingOrderWithTheirViewportAsBounds()
    {
        var tree = new PaneTree();
        int right = tree.SplitFocused(SplitAxis.SideBySide);
        tree.Focus(0);
        int below = tree.SplitFocused(SplitAxis.Stacked);
        PaneViewport[] layout = [.. tree.Layout(801, 601, divider: 1)];

        IReadOnlyList<PaneElement> elements = PaneElements.Describe(tree, layout);

        Assert.AreEqual($"0,{below},{right}", string.Join(",", elements.Select(e => e.Pane)));
        Assert.AreEqual("1,2,3", string.Join(",", elements.Select(e => e.Number)));
        Assert.AreEqual("Terminal, pane 1|Terminal, pane 2|Terminal, pane 3", string.Join("|", elements.Select(e => e.Name)));
        foreach (PaneElement e in elements)
        {
            PaneViewport v = layout.Single(l => l.Pane == e.Pane);
            Assert.AreEqual(new PixelRect(v.X, v.Y, v.Width, v.Height), e.Bounds);
        }
    }

    [TestMethod]
    public void OnlyTheFocusedPaneIsFocusedAndFocusMovesIt()
    {
        var tree = new PaneTree();
        int right = tree.SplitFocused(SplitAxis.SideBySide);
        PaneViewport[] layout = [.. tree.Layout(800, 600)];

        Assert.AreEqual(right, PaneElements.Describe(tree, layout).Single(e => e.Focused).Pane);

        tree.Focus(0);
        Assert.AreEqual(0, PaneElements.Describe(tree, layout).Single(e => e.Focused).Pane);
    }

    [TestMethod]
    public void ANumberFollowsThePositionNotThePaneId()
    {
        var tree = new PaneTree();
        int b = tree.SplitFocused(SplitAxis.SideBySide);
        tree.Focus(0);
        Assert.IsTrue(tree.Close(0));

        PaneElement left = PaneElements.Describe(tree, tree.Layout(800, 600)).Single();

        Assert.AreEqual(b, left.Pane);
        Assert.AreEqual("Terminal, pane 1", left.Name, "the survivor is now the first pane");
    }

    [TestMethod]
    public void APaneBeforeTheFirstLayoutHasEmptyBounds()
    {
        var tree = new PaneTree();
        tree.SplitFocused(SplitAxis.Stacked);

        IReadOnlyList<PaneElement> elements = PaneElements.Describe(tree, []);

        Assert.AreEqual(2, elements.Count);
        Assert.IsTrue(elements.All(e => e.Bounds == default));
    }

    [TestMethod]
    public void FocusIsAnnouncedOnceUntilItMovesOrIsLost()
    {
        var tracker = new PaneFocusTracker();

        Assert.IsNull(tracker.Observe(hostFocused: false, pane: 0), "the host has no focus yet");
        Assert.AreEqual(0, tracker.Observe(hostFocused: true, pane: 0));
        Assert.IsNull(tracker.Observe(hostFocused: true, pane: 0), "already told");
        Assert.AreEqual(2, tracker.Observe(hostFocused: true, pane: 2), "moved to another pane");
        Assert.AreEqual(0, tracker.Observe(hostFocused: true, pane: 0), "pane 0 again after pane 2, not a repeat");

        Assert.IsNull(tracker.Observe(hostFocused: false, pane: 0));
        Assert.AreEqual(0, tracker.Observe(hostFocused: true, pane: 0), "regaining focus announces again");
        Assert.IsNull(tracker.Observe(hostFocused: true, pane: null), "no pane, nothing to announce");
        Assert.AreEqual(0, tracker.Observe(hostFocused: true, pane: 0), "and a pane back is announced");
    }
}
