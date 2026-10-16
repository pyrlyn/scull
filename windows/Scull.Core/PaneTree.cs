namespace Scull.Core;

// The layout of one window's panes as a value: a tree whose leaves are panes
// and whose nodes split the space in two. It mirrors macos/Sources/ScullKit/
// PaneTree.swift so both platforms split, close and move focus alike. No
// WinUI types and no core handles live here: on Windows the leaves become
// viewports of the window's single swap chain, see PaneTree.Layout.

/// <summary>How a split places its two panes.</summary>
public enum SplitAxis
{
    /// <summary>The panes sit side by side.</summary>
    SideBySide,

    /// <summary>The panes sit one above the other.</summary>
    Stacked,
}

/// <summary>A pane's share of the surface, in pixels from its top-left corner.</summary>
public readonly record struct PaneViewport(int Pane, int X, int Y, int Width, int Height)
{
    /// <summary>Whether the pixel (<paramref name="x"/>, <paramref name="y"/>) is inside; the right and bottom edges are not.</summary>
    public bool Contains(double x, double y) => x >= X && x < X + Width && y >= Y && y < Y + Height;
}

/// <summary>A leaf (a pane) or a split of two subtrees.</summary>
public abstract record PaneNode
{
    private PaneNode()
    {
    }

    /// <summary>A pane.</summary>
    public sealed record Leaf(int Id) : PaneNode;

    /// <summary>Two subtrees along an axis; <paramref name="First"/> is left or top.</summary>
    public sealed record Split(SplitAxis Axis, PaneNode First, PaneNode Second) : PaneNode;

    /// <summary>The panes in reading order: left to right, top to bottom.</summary>
    public IEnumerable<int> Leaves() => this switch
    {
        Leaf leaf => [leaf.Id],
        Split split => split.First.Leaves().Concat(split.Second.Leaves()),
        _ => [],
    };

    internal PaneNode? Removing(int id) => this switch
    {
        Leaf leaf => leaf.Id == id ? null : this,
        // The sibling takes the closed pane's whole share.
        Split split => (split.First.Removing(id), split.Second.Removing(id)) switch
        {
            (null, var rest) => rest,
            (var rest, null) => rest,
            (var first, var second) => new Split(split.Axis, first!, second!),
        },
        _ => this,
    };

    internal PaneNode Splitting(int id, SplitAxis axis, int created) => this switch
    {
        Leaf leaf => leaf.Id == id ? new Split(axis, this, new Leaf(created)) : this,
        Split split => new Split(
            split.Axis,
            split.First.Splitting(id, axis, created),
            split.Second.Splitting(id, axis, created)),
        _ => this,
    };

    internal void Place(int x, int y, int width, int height, int divider, List<PaneViewport> into)
    {
        switch (this)
        {
            case Leaf leaf:
                into.Add(new PaneViewport(leaf.Id, x, y, width, height));
                break;
            case Split { Axis: SplitAxis.SideBySide } split:
                {
                    // The first pane gets the rounded-down half so the sizes never drift between redraws.
                    // A surface narrower than the divider has no room for it, and nothing may start past the edge.
                    int gap = Math.Min(divider, width);
                    int room = width - gap;
                    int first = room / 2;
                    split.First.Place(x, y, first, height, divider, into);
                    split.Second.Place(x + first + gap, y, room - first, height, divider, into);
                    break;
                }

            case Split split:
                {
                    int gap = Math.Min(divider, height);
                    int room = height - gap;
                    int first = room / 2;
                    split.First.Place(x, y, width, first, divider, into);
                    split.Second.Place(x, y + first + gap, width, room - first, divider, into);
                    break;
                }
        }
    }
}

/// <summary>One window's panes and the focused one.</summary>
public sealed class PaneTree
{
    private int nextId = 1;

    /// <summary>A tree of the single pane 0, which is focused.</summary>
    public PaneTree()
    {
        Root = new PaneNode.Leaf(0);
        Focused = 0;
    }

    /// <summary>The layout; replaced, never edited, by every change.</summary>
    public PaneNode Root { get; private set; }

    /// <summary>The pane that takes keys.</summary>
    public int Focused { get; private set; }

    /// <summary>The id the next split will give its new pane; ids are never reused.</summary>
    public int NextId => nextId;

    /// <summary>The panes in reading order.</summary>
    public IReadOnlyList<int> Panes => [.. Root.Leaves()];

    /// <summary>Splits the focused pane, puts the new pane after it and focuses it.</summary>
    public int SplitFocused(SplitAxis axis)
    {
        int id = nextId++;
        Root = Root.Splitting(Focused, axis, id);
        Focused = id;
        return id;
    }

    /// <summary>
    /// Removes a pane. The pane after it, or the one before when it was last, takes the
    /// focus if it had it. False when <paramref name="id"/> is not here or is the last
    /// pane; closing the last one is the window's job.
    /// </summary>
    public bool Close(int id)
    {
        IReadOnlyList<int> order = Panes;
        int index = IndexOf(order, id);
        if (index < 0 || order.Count < 2 || Root.Removing(id) is not { } rest)
        {
            return false;
        }

        Root = rest;
        if (Focused == id)
        {
            Focused = index + 1 < order.Count ? order[index + 1] : order[index - 1];
        }

        return true;
    }

    /// <summary>Focuses <paramref name="id"/> if it is a pane here.</summary>
    public void Focus(int id)
    {
        if (IndexOf(Panes, id) >= 0)
        {
            Focused = id;
        }
    }

    /// <summary>Moves the focus along the reading order, wrapping around.</summary>
    public void FocusNext(int step = 1)
    {
        IReadOnlyList<int> order = Panes;
        int index = IndexOf(order, Focused);
        if (index >= 0)
        {
            Focused = order[(index + (step % order.Count) + order.Count) % order.Count];
        }
    }

    /// <summary>
    /// Cuts a <paramref name="width"/> x <paramref name="height"/> pixel surface into one
    /// viewport per pane, in reading order, leaving <paramref name="divider"/> pixels
    /// between neighbours. The window draws every viewport into its one swap chain.
    /// </summary>
    public IReadOnlyList<PaneViewport> Layout(int width, int height, int divider = 0)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(width);
        ArgumentOutOfRangeException.ThrowIfNegative(height);
        ArgumentOutOfRangeException.ThrowIfNegative(divider);
        var viewports = new List<PaneViewport>();
        Root.Place(0, 0, width, height, divider, viewports);
        return viewports;
    }

    /// <summary>The pane under a pixel of a <see cref="Layout"/>, or null over a divider or outside.</summary>
    public static int? PaneAt(IEnumerable<PaneViewport> viewports, double x, double y)
    {
        foreach (PaneViewport viewport in viewports)
        {
            if (viewport.Contains(x, y))
            {
                return viewport.Pane;
            }
        }

        return null;
    }

    private static int IndexOf(IReadOnlyList<int> order, int id)
    {
        for (int i = 0; i < order.Count; i++)
        {
            if (order[i] == id)
            {
                return i;
            }
        }

        return -1;
    }
}
