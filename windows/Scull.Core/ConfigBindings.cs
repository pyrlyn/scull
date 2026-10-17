namespace Scull.Core;

/// <summary>The configured key bindings, matched as TerminalView.swift's <c>runBinding</c> matches them.</summary>
public static class ConfigBindings
{
    private const KeyModifiers Shortcut = KeyModifiers.Shift | KeyModifiers.Alt | KeyModifiers.Control | KeyModifiers.Super;

    /// <summary>
    /// The action bound to <paramref name="key"/> (a <see cref="KeyCode"/> from
    /// <see cref="WindowsKeys.Map"/>) with exactly the shortcut modifiers held;
    /// Caps Lock and Num Lock never stop a binding from matching.
    /// </summary>
    public static ConfigAction? Match(IReadOnlyList<ConfigKeybind> binds, uint? key, KeyModifiers held)
    {
        ArgumentNullException.ThrowIfNull(binds);
        if (key is not uint pressed)
        {
            return null;
        }
        KeyModifiers mods = held & Shortcut;
        foreach (ConfigKeybind bind in binds)
        {
            if (bind.Key == pressed && bind.Mods == mods)
            {
                return bind.Action;
            }
        }
        return null;
    }

    /// <summary>
    /// The rows a scroll action moves the viewport back into history, for
    /// <see cref="Terminal.ScrollDisplay"/>, on a grid of <paramref name="rows"/>; null for other actions.
    /// </summary>
    public static int? ScrollRows(ConfigAction action, int rows) => action switch
    {
        ConfigAction.ScrollPageUp => rows,
        ConfigAction.ScrollPageDown => -rows,
        // The core clamps, so the extremes reach either end of any history.
        ConfigAction.ScrollToTop => int.MaxValue,
        ConfigAction.ScrollToBottom => -int.MaxValue,
        _ => null,
    };
}

/// <summary>
/// One view's font: the configured family and size plus the view's own zoom,
/// as TerminalView.swift keeps them. A change of the configured font drops the zoom.
/// </summary>
public sealed class FontZoom
{
    public const float MinSize = 4, MaxSize = 200;

    // Tried after the configured family; Consolas ships with every Windows, Server included.
    private static readonly string[] Fallbacks = ["Cascadia Mono", "Consolas"];

    private string family;
    private float size;
    private float zoom;

    /// <param name="family">Empty for the system's monospace font.</param>
    public FontZoom(string family = "", float size = 14)
    {
        this.family = family ?? "";
        this.size = float.IsFinite(size) ? size : 14;
    }

    /// <summary>The size to draw at, in device-independent pixels.</summary>
    public float Size => Math.Clamp(size + zoom, MinSize, MaxSize);

    /// <summary>The families to try in turn: the configured one, then the system's monospace fonts.</summary>
    public IReadOnlyList<string> Families => family.Length == 0 ? Fallbacks : [family, .. Fallbacks];

    /// <summary>Takes the configured font; true when it differs from the one in use.</summary>
    public bool Apply(ConfigSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        if (settings.FontFamily == family && settings.FontSize == size)
        {
            return false;
        }
        family = settings.FontFamily;
        size = float.IsFinite(settings.FontSize) ? settings.FontSize : size;
        zoom = 0;
        return true;
    }

    /// <summary>Applies a font action; true when the size changed, false for any other action or at a limit.</summary>
    public bool Run(ConfigAction action)
    {
        float before = Size;
        zoom = action switch
        {
            ConfigAction.FontLarger => zoom + 1,
            ConfigAction.FontSmaller => zoom - 1,
            ConfigAction.FontReset => 0,
            _ => zoom,
        };
        // Held inside the limits, so the first step back from one takes effect at once.
        zoom = Size - size;
        return Size != before;
    }
}
