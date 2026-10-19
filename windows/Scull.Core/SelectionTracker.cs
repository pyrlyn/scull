namespace Scull.Core;

/// <summary>
/// Decides when a pane's selection (or, with none, its caret) moved in a way a UI Automation
/// client must hear of. Like <see cref="TextChangeTracker"/> it reads nothing while nobody listens.
/// </summary>
public sealed class SelectionTracker
{
    private TextRange? last;

    /// <summary>
    /// True when the range differs from the previous call's. The first call after a stretch with no
    /// listener only takes a baseline: the client reads the selection itself when it starts. <paramref name="read"/>
    /// is not called while nobody <paramref name="listening"/>.
    /// </summary>
    public bool Observe(bool listening, Func<TextRange> read)
    {
        ArgumentNullException.ThrowIfNull(read);
        if (!listening)
        {
            last = null;
            return false;
        }
        TextRange next = read();
        bool changed = last is { } before && before != next;
        last = next;
        return changed;
    }
}
