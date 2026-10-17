using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using Scull.Native;

namespace Scull.Core;

/// <summary>What a key binding does; the values are the core's <c>TT_ACTION_*</c>.</summary>
public enum ConfigAction : byte
{
    Paste = NativeMethods.TT_ACTION_PASTE,
    FontLarger = NativeMethods.TT_ACTION_FONT_LARGER,
    FontSmaller = NativeMethods.TT_ACTION_FONT_SMALLER,
    FontReset = NativeMethods.TT_ACTION_FONT_RESET,
    ScrollPageUp = NativeMethods.TT_ACTION_SCROLL_PAGE_UP,
    ScrollPageDown = NativeMethods.TT_ACTION_SCROLL_PAGE_DOWN,
    ScrollToTop = NativeMethods.TT_ACTION_SCROLL_TO_TOP,
    ScrollToBottom = NativeMethods.TT_ACTION_SCROLL_TO_BOTTOM,
}

/// <summary>Do <paramref name="Action"/> when <paramref name="Key"/> is pressed with exactly <paramref name="Mods"/> held.</summary>
/// <param name="Key">A <see cref="KeyCode"/>.</param>
public readonly record struct ConfigKeybind(uint Key, KeyModifiers Mods, ConfigAction Action);

/// <summary>
/// The settings in force as one poll saw them, copied out of the core's buffers
/// so they outlive the next poll. Colours are <c>0xRRGGBB</c>.
/// </summary>
public sealed record ConfigSettings
{
    /// <summary>Grows with every change of the settings or of <see cref="Error"/>.</summary>
    public ulong Generation { get; init; }

    /// <summary>False means only <see cref="Config.Set"/> and a new handle see the file.</summary>
    public bool Watching { get; init; }

    /// <summary>Empty for the system's monospace font.</summary>
    public string FontFamily { get; init; } = "";

    public float FontSize { get; init; }

    /// <summary>History rows for terminals opened from now on; running ones keep theirs.</summary>
    public uint Scrollback { get; init; }

    public string Scheme { get; init; } = "";

    public uint Foreground { get; init; }

    public uint Background { get; init; }

    public uint Cursor { get; init; }

    /// <summary>Palette entries 0 to 15.</summary>
    public IReadOnlyList<uint> Ansi { get; init; } = [];

    public IReadOnlyList<ConfigKeybind> Keybinds { get; init; } = [];

    /// <summary>Why the file was refused, with its name; empty when it was fine. The rest is then the last good settings.</summary>
    public string Error { get; init; } = "";

    public string Path { get; init; } = "";
}

/// <summary>
/// One <c>tt_config</c>: the user's config file, watched by the core. Like
/// <see cref="Terminal"/> it is used from one thread at a time, the UI thread;
/// only Dispose may come from elsewhere.
/// </summary>
public sealed unsafe class Config : IDisposable
{
    private readonly NativeHandle config;

    private Config(NativeHandle config) => this.config = config;

    /// <summary>The settings as the last <see cref="TryPoll"/> that saw a change left them; null before the first.</summary>
    public ConfigSettings? Settings { get; private set; }

    /// <summary>
    /// Opens the file at <paramref name="path"/>, or the default place when null or
    /// empty, and watches it. A missing file is the defaults and a broken one an
    /// <see cref="ConfigSettings.Error"/>. <paramref name="wakeup"/> runs on a core
    /// thread when <see cref="TryPoll"/> would answer something new; it must not
    /// block, throw or call this config, only hand the work to the UI thread. It is
    /// not called again until that thread polls, and never after <see cref="Dispose"/>.
    /// </summary>
    public static Config Open(string? path, Action wakeup)
    {
        ArgumentNullException.ThrowIfNull(wakeup);
        Abi.EnsureCompatible();
        byte[] bytes = Encoding.UTF8.GetBytes(path ?? "");
        GCHandle userdata = GCHandle.Alloc(wakeup);
        tt_config* raw = null;
        tt_status status;
        fixed (byte* p = bytes)
        {
            var options = new tt_config_options
            {
                struct_size = (uint)sizeof(tt_config_options),
                abi_version = NativeMethods.TT_ABI_VERSION,
                path = new tt_str { ptr = p, len = (nuint)bytes.Length },
                wakeup = &Wake,
                userdata = (void*)GCHandle.ToIntPtr(userdata),
            };
            status = NativeMethods.tt_config_new(&options, &raw);
        }
        if (status != tt_status.TT_OK)
        {
            userdata.Free();
            throw ScullException.From(status, "tt_config_new");
        }
        return new Config(new NativeHandle((nint)raw, &Free, userdata));
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void Wake(void* userdata) => ((Action)GCHandle.FromIntPtr((nint)userdata).Target!)();

    /// <summary>
    /// Reads the core's settings. True, with <see cref="Settings"/> replaced, when
    /// anything changed since the previous poll (always the first); call it after
    /// every wakeup, which is what lets the next one fire.
    /// </summary>
    public bool TryPoll(out ConfigSettings settings)
    {
        var view = new tt_config_view { struct_size = (uint)sizeof(tt_config_view) };
        using (var c = new Lease(config))
        {
            Check(NativeMethods.tt_config_poll((tt_config*)c.Pointer, &view), "tt_config_poll");
        }
        if (view.updated == 0 && Settings is { } current)
        {
            settings = current;
            return false;
        }
        settings = Settings = Copy(view);
        return view.updated != 0;
    }

    /// <summary>
    /// Changes one setting in the file (<c>font.family</c>, <c>font.size</c>,
    /// <c>colors.scheme</c>, <c>colors.foreground</c>, <c>colors.background</c>,
    /// <c>colors.cursor</c>, <c>scrollback</c>) and keeps the rest of it; an empty
    /// <paramref name="value"/> removes the key so its default applies. The file is
    /// untouched and <see cref="Status.Invalid"/> is thrown for an unknown key or a
    /// value the config refuses, <see cref="Status.Io"/> when it cannot be written.
    /// The next <see cref="TryPoll"/> sees the change.
    /// </summary>
    public void Set(string key, string value)
    {
        ArgumentNullException.ThrowIfNull(key);
        ArgumentNullException.ThrowIfNull(value);
        byte[] keyBytes = Encoding.UTF8.GetBytes(key), valueBytes = Encoding.UTF8.GetBytes(value);
        using var c = new Lease(config);
        fixed (byte* k = keyBytes)
        fixed (byte* v = valueBytes)
        {
            Check(
                NativeMethods.tt_config_set(
                    (tt_config*)c.Pointer,
                    new tt_str { ptr = k, len = (nuint)keyBytes.Length },
                    new tt_str { ptr = v, len = (nuint)valueBytes.Length }),
                "tt_config_set");
        }
    }

    /// <summary>Stops the watcher and frees the config. A second call does nothing.</summary>
    public void Dispose() => config.Dispose();

    private static ConfigSettings Copy(tt_config_view view)
    {
        var binds = new List<ConfigKeybind>((int)view.keybinds_len);
        for (nuint i = 0; i < view.keybinds_len; i++)
        {
            tt_keybind bind = view.keybinds[i];
            // Actions are only appended, so a host skips ones it does not know.
            if (Enum.IsDefined((ConfigAction)bind.action))
            {
                binds.Add(new ConfigKeybind(bind.key, (KeyModifiers)bind.mods, (ConfigAction)bind.action));
            }
        }
        return new ConfigSettings
        {
            Generation = view.generation,
            Watching = view.watching != 0,
            FontFamily = Text(view.font_family),
            FontSize = view.font_size,
            Scrollback = view.scrollback,
            Scheme = Text(view.scheme),
            Foreground = view.foreground,
            Background = view.background,
            Cursor = view.cursor,
            Ansi = new ReadOnlySpan<uint>(view.ansi, 16).ToArray(),
            Keybinds = binds,
            Error = Text(view.error),
            Path = Text(view.path),
        };
    }

    private static string Text(tt_str s) => s.ptr == null ? "" : Encoding.UTF8.GetString(s.ptr, checked((int)s.len));

    private static void Check(tt_status status, string call)
    {
        if (status != tt_status.TT_OK)
        {
            throw ScullException.From(status, call);
        }
    }

    private static void Free(nint pointer) => NativeMethods.tt_config_free((tt_config*)pointer);
}
