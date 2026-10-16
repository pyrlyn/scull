namespace Scull.Core;

/// <summary>What a window does with its tabs and panes.</summary>
public enum PaneCommand
{
    NewTab,
    NewWindow,
    SplitRight,
    SplitDown,
    ClosePane,
    NextPane,
    PreviousPane,
}

/// <summary>The keys that run a <see cref="PaneCommand"/>, so the program never sees them.</summary>
public static class PaneCommands
{
    private const KeyModifiers Chord = KeyModifiers.Shift | KeyModifiers.Alt | KeyModifiers.Control | KeyModifiers.Super;

    /// <summary>
    /// The command for a virtual key with these modifiers held: Ctrl+Shift with T, N, D, E, W, ] or [.
    /// Ctrl+Shift is the terminal-safe prefix (paste is Ctrl+Shift+V), and the letters follow Windows Terminal where it has one.
    /// </summary>
    public static PaneCommand? Match(int virtualKey, KeyModifiers held) =>
        (held & Chord) != (KeyModifiers.Control | KeyModifiers.Shift) ? null : virtualKey switch
        {
            0x54 => PaneCommand.NewTab, // T
            0x4E => PaneCommand.NewWindow, // N
            0x44 => PaneCommand.SplitRight, // D
            0x45 => PaneCommand.SplitDown, // E
            0x57 => PaneCommand.ClosePane, // W
            0xDD => PaneCommand.NextPane, // VK_OEM_6, ]
            0xDB => PaneCommand.PreviousPane, // VK_OEM_4, [
            _ => null,
        };
}
