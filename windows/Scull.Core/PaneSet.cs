namespace Scull.Core;

/// <summary>
/// One window's <see cref="PaneTree"/> with a <see cref="Terminal"/> per pane. A pane
/// keeps its terminal through every split and close of its neighbours, so a split never
/// restarts a shell, and a poisoned terminal poisons its own pane only.
/// </summary>
public sealed class PaneSet : IDisposable
{
    private readonly Func<int, Terminal> create;
    private readonly Dictionary<int, Terminal> terminals = [];
    private readonly PaneTree tree = new();

    /// <param name="create">Makes the terminal of a pane, given its id; called once per pane.</param>
    public PaneSet(Func<int, Terminal> create)
    {
        ArgumentNullException.ThrowIfNull(create);
        this.create = create;
        terminals[0] = create(0);
    }

    /// <summary>The layout; read it, change it only through this set.</summary>
    public PaneTree Tree => tree;

    /// <summary>The terminal of a pane here.</summary>
    public Terminal this[int pane] => terminals[pane];

    /// <summary>The panes whose terminal the core has poisoned, in reading order.</summary>
    public IEnumerable<int> Poisoned => tree.Panes.Where(p => terminals[p].IsPoisoned);

    /// <summary>
    /// Splits the focused pane; the new pane's terminal is made first, so a failure
    /// leaves the tree as it was.
    /// </summary>
    public int Split(SplitAxis axis)
    {
        Terminal terminal = create(tree.NextId);
        int id = tree.SplitFocused(axis);
        terminals[id] = terminal;
        return id;
    }

    /// <summary>
    /// Closes a pane and frees its terminal. False when it is not here or is the last
    /// pane, which only the window can close.
    /// </summary>
    public bool Close(int pane)
    {
        if (!tree.Close(pane))
        {
            return false;
        }

        terminals.Remove(pane, out Terminal? gone);
        gone!.Dispose();
        return true;
    }

    /// <summary>Frees every terminal.</summary>
    public void Dispose()
    {
        foreach (Terminal terminal in terminals.Values)
        {
            terminal.Dispose();
        }

        terminals.Clear();
    }
}
