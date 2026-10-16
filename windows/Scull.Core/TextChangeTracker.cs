namespace Scull.Core;

/// <summary>
/// Decides when a pane's text changed in a way a UI Automation client must hear of.
/// Reading and comparing the whole text on every frame costs something a terminal
/// that nobody is listening to should not pay, so the text is read only while a
/// client listens.
/// </summary>
public sealed class TextChangeTracker
{
    private ScreenText? last;

    /// <summary>
    /// What changed since the previous call, or null for nothing. The first call after a
    /// stretch with no listener reports every line, since the client has seen none of it.
    /// <paramref name="read"/> is not called while nobody <paramref name="listening"/>.
    /// </summary>
    public TextChange? Observe(bool listening, Func<ScreenText> read)
    {
        ArgumentNullException.ThrowIfNull(read);
        if (!listening)
        {
            last = null;
            return null;
        }
        ScreenText next = read();
        TextChange? change = TextChange.Between(last, next);
        last = next;
        return change;
    }
}
