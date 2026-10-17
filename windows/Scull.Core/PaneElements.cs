namespace Scull.Core;

/// <summary>
/// One pane as a UI Automation client sees it: a text element of its own. <see cref="Bounds"/>
/// are in the swap chain's pixels; the host adds the swap chain's place on the screen.
/// </summary>
/// <param name="Pane">The pane's id, which outlives splits and closes of its neighbours.</param>
/// <param name="Number">The pane's place in reading order, from 1, which is what its name says.</param>
public readonly record struct PaneElement(int Pane, int Number, string Name, PixelRect Bounds, bool Focused);

/// <summary>
/// The panes of a tab as automation elements. The panes are regions of one swap chain, not
/// controls, so nothing in the UI describes them; this does, from the tree and its layout, and
/// the host turns each element into a child peer.
/// </summary>
public static class PaneElements
{
    /// <summary>The panes in reading order, the same order focus moves in and the layout lists them.</summary>
    /// <param name="layout">The viewports of <see cref="PaneTree.Layout"/>; a pane it lacks (before the first layout) has empty bounds.</param>
    public static IReadOnlyList<PaneElement> Describe(PaneTree tree, IEnumerable<PaneViewport> layout)
    {
        ArgumentNullException.ThrowIfNull(tree);
        ArgumentNullException.ThrowIfNull(layout);
        Dictionary<int, PaneViewport> viewports = layout.ToDictionary(v => v.Pane);
        List<PaneElement> elements = [];
        foreach (int pane in tree.Panes)
        {
            PixelRect bounds = viewports.TryGetValue(pane, out PaneViewport v) ? new PixelRect(v.X, v.Y, v.Width, v.Height) : default;
            elements.Add(new PaneElement(pane, elements.Count + 1, $"Terminal, pane {elements.Count + 1}", bounds, pane == tree.Focused));
        }
        return elements;
    }
}

/// <summary>
/// Decides when the keyboard focus has to be announced on a pane. Moving the focus between panes
/// moves nothing in XAML, so a screen reader only learns of it when the host says which child now
/// has it; announcing a pane it was already told about would make it read the pane again.
/// </summary>
public sealed class PaneFocusTracker
{
    private int? announced;

    /// <summary>
    /// The pane to announce, or null for none. <paramref name="pane"/> is the pane that takes the
    /// typing; it counts only while the host has the keyboard focus, and losing that forgets the
    /// last announcement so regaining it announces again.
    /// </summary>
    public int? Observe(bool hostFocused, int? pane)
    {
        int? now = hostFocused ? pane : null;
        if (now == announced)
        {
            return null;
        }
        announced = now;
        return now;
    }
}
