namespace Scull.Core;

/// <summary>
/// The link one pane underlines while Ctrl is held, as TerminalView+Links.swift keeps it on macOS.
/// </summary>
public sealed class LinkHover
{
    /// <summary>The id last given to the core; it may since have been dropped there.</summary>
    public uint? Hovered { get; private set; }

    /// <summary>
    /// Points <paramref name="terminal"/>'s hover at the link in viewport cell <paramref name="at"/> while
    /// <paramref name="held"/> has Ctrl, else at none; true when the core was told, and the frame wants an update.
    /// </summary>
    public bool Follow(Terminal terminal, (ushort Row, ushort Col)? at, KeyModifiers held)
    {
        uint? id = (held & KeyModifiers.Control) != 0 && at is { } cell ? terminal.LinkAt(cell.Row, cell.Col) : null;
        // The core may have dropped the cached id and handed it to another link, so only
        // "none, and still none" skips telling it.
        if (id is null && Hovered is null)
        {
            return false;
        }
        Hovered = id;
        terminal.HoverLink(id);
        return true;
    }
}
