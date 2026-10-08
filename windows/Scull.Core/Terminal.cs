using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using Scull.Native;

namespace Scull.Core;

/// <summary>
/// One <c>tt_term</c> and the <c>tt_frame</c> that reads it. Like the frame,
/// it is used from one thread at a time, the UI thread; only Dispose may come
/// from elsewhere, and the handles make a call in flight finish before the free.
/// </summary>
public sealed unsafe class Terminal : IDisposable
{
    // Checked once per process; a race only checks twice.
    private static volatile bool abiChecked;

    private readonly NativeHandle term;
    private readonly NativeHandle frame;
    private tt_frame_view view;

    /// <summary>A terminal with no child: bytes reach it only through <see cref="Feed"/>.</summary>
    public Terminal(ushort cols, ushort rows, uint scrollback = 10_000)
        : this(New(cols, rows, scrollback), default)
    {
    }

    private Terminal(nint rawTerm, GCHandle userdata)
    {
        term = new NativeHandle(rawTerm, &FreeTerm, userdata);
        tt_frame* rawFrame = NativeMethods.tt_frame_new();
        if (rawFrame == null)
        {
            term.Dispose();
            throw new ScullException(Status.Panic, "tt_frame_new failed inside the core");
        }
        frame = new NativeHandle((nint)rawFrame, &FreeFrame);
    }

    /// <summary>
    /// A terminal running a child on a new PTY: <see cref="SpawnOptions.Program"/>,
    /// or the user's shell. <paramref name="wakeup"/> runs on a core thread when
    /// there is output or an event; it must not block, throw or call this terminal,
    /// only hand the work to the UI thread, which then polls and updates. It is not
    /// called again until that happens, and never after <see cref="Dispose"/> has freed the terminal.
    /// </summary>
    public static Terminal Spawn(ushort cols, ushort rows, Action wakeup, SpawnOptions? spawn = null)
    {
        ArgumentNullException.ThrowIfNull(wakeup);
        spawn ??= new SpawnOptions();
        EnsureAbi();
        // One buffer for every string, so a single pin covers them for the call.
        string[] strings = [spawn.Program ?? "", spawn.WorkingDirectory ?? "", .. spawn.Arguments, .. spawn.Environment];
        var ends = new int[strings.Length];
        var bytes = new byte[strings.Sum(Encoding.UTF8.GetByteCount)];
        for (int i = 0, at = 0; i < strings.Length; i++)
        {
            at += Encoding.UTF8.GetBytes(strings[i], bytes.AsSpan(at));
            ends[i] = at;
        }
        var strs = new tt_str[strings.Length];
        GCHandle userdata = GCHandle.Alloc(wakeup);
        tt_term* raw = null;
        tt_status status;
        fixed (byte* b = bytes)
        fixed (tt_str* s = strs)
        {
            for (int i = 0, start = 0; i < strings.Length; start = ends[i++])
            {
                s[i] = new tt_str { ptr = b + start, len = (nuint)(ends[i] - start) };
            }
            tt_term_options options = Options(cols, rows, spawn.Scrollback);
            (options.program, options.cwd) = (s[0], s[1]);
            options.args = s + 2;
            options.args_len = (nuint)spawn.Arguments.Count;
            options.env = s + 2 + spawn.Arguments.Count;
            options.env_len = (nuint)spawn.Environment.Count;
            options.wakeup = &Wake;
            options.userdata = (void*)GCHandle.ToIntPtr(userdata);
            status = NativeMethods.tt_term_spawn(&options, &raw);
        }
        if (status != tt_status.TT_OK)
        {
            userdata.Free();
            throw ScullException.From(status, "tt_term_spawn");
        }
        return new Terminal((nint)raw, userdata);
    }

    private static nint New(ushort cols, ushort rows, uint scrollback)
    {
        EnsureAbi();
        tt_term_options options = Options(cols, rows, scrollback);
        tt_term* raw = null;
        tt_status status = NativeMethods.tt_term_new(&options, &raw);
        if (status != tt_status.TT_OK)
        {
            throw ScullException.From(status, "tt_term_new");
        }
        return (nint)raw;
    }

    private static void EnsureAbi()
    {
        if (!abiChecked)
        {
            Abi.EnsureCompatible();
            abiChecked = true;
        }
    }

    private static tt_term_options Options(ushort cols, ushort rows, uint scrollback) => new()
    {
        struct_size = (uint)sizeof(tt_term_options),
        abi_version = NativeMethods.TT_ABI_VERSION,
        cols = cols,
        rows = rows,
        scrollback = scrollback,
    };

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void Wake(void* userdata) => ((Action)GCHandle.FromIntPtr((nint)userdata).Target!)();

    /// <summary>
    /// True once a call answered <see cref="Status.Poisoned"/> or <see cref="Status.Panic"/>:
    /// the core hit a bug in this terminal and only disposing it is left.
    /// </summary>
    public bool IsPoisoned { get; private set; }

    /// <summary>The frame as the last <see cref="Update"/> left it; empty before the first.</summary>
    public FrameView View
    {
        get
        {
            ObjectDisposedException.ThrowIf(frame.IsClosed, this);
            return new FrameView(view);
        }
    }

    /// <summary>Parses program output, as if a child had written it.</summary>
    public void Feed(ReadOnlySpan<byte> bytes)
    {
        using var t = new Lease(term);
        fixed (byte* p = bytes)
        {
            Check(NativeMethods.tt_term_feed((tt_term*)t.Pointer, p, (nuint)bytes.Length), "tt_term_feed");
        }
    }

    /// <summary>Holds the child's output back during an interactive resize, until <see cref="Resize"/>.</summary>
    public void ResizeBegin()
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_resize_begin((tt_term*)t.Pointer), "tt_term_resize_begin");
    }

    /// <summary>
    /// Resizes to <paramref name="cols"/> x <paramref name="rows"/> cells; the pixel
    /// size of the view, 0 when unknown, also gives the cell size images are measured with.
    /// </summary>
    public void Resize(ushort cols, ushort rows, ushort widthPx = 0, ushort heightPx = 0)
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_resize((tt_term*)t.Pointer, cols, rows, widthPx, heightPx), "tt_term_resize");
    }

    /// <summary>
    /// Sends a key, encoded for the program's key modes. False when nothing could
    /// be sent: there is no child, or it is not reading and its input queue is full.
    /// </summary>
    public bool Key(in KeyInput key)
    {
        ArgumentNullException.ThrowIfNull(key.Text);
        byte[] bytes = Encoding.UTF8.GetBytes(key.Text);
        using var t = new Lease(term);
        fixed (byte* p = bytes)
        {
            var raw = new tt_key_event
            {
                struct_size = (uint)sizeof(tt_key_event),
                key = key.Key,
                action = (byte)key.Action,
                mods = (byte)key.Modifiers,
                consumed_mods = (byte)key.Consumed,
                text = new tt_str { ptr = p, len = (nuint)bytes.Length },
            };
            return Sent(NativeMethods.tt_term_key((tt_term*)t.Pointer, &raw), "tt_term_key");
        }
    }

    /// <summary>Sends text committed outside a key event; control characters are dropped. False as for <see cref="Key"/>.</summary>
    public bool Text(string text) => SendText(text, &NativeMethods.tt_term_text, "tt_term_text");

    /// <summary>
    /// Sends pasted text, bracketed while the program asks for it. A paste goes whole
    /// or not at all, so one larger than the child's input queue is false, as for <see cref="Key"/>.
    /// </summary>
    public bool Paste(string text) => SendText(text, &NativeMethods.tt_term_paste, "tt_term_paste");

    /// <summary>Tells the program the view gained or lost focus, if it asked to know. False as for <see cref="Key"/>.</summary>
    public bool Focus(bool focused)
    {
        using var t = new Lease(term);
        return Sent(NativeMethods.tt_term_focus((tt_term*)t.Pointer, focused ? (byte)1 : (byte)0), "tt_term_focus");
    }

    /// <summary>
    /// Sends a mouse event if the program takes it. True when it did, even if the
    /// bytes could not be written, so the host never acts under a program that
    /// tracks the mouse; false leaves the event to the host (scroll the history).
    /// </summary>
    public bool Mouse(in MouseInput mouse)
    {
        var raw = new tt_mouse_event
        {
            struct_size = (uint)sizeof(tt_mouse_event),
            action = (byte)mouse.Action,
            button = (byte)mouse.Button,
            mods = (byte)mouse.Modifiers,
            col = mouse.Col,
            row = mouse.Row,
            x_px = mouse.XPx,
            y_px = mouse.YPx,
        };
        byte taken = 0;
        using var t = new Lease(term);
        Sent(NativeMethods.tt_term_mouse((tt_term*)t.Pointer, &raw, &taken), "tt_term_mouse");
        return taken != 0;
    }

    /// <summary>Moves the viewport <paramref name="rows"/> back into history (negative: towards the screen), clamped.</summary>
    public void ScrollDisplay(int rows)
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_scroll_display((tt_term*)t.Pointer, rows), "tt_term_scroll_display");
    }

    private bool SendText(string text, delegate*<tt_term*, byte*, nuint, tt_status> send, string call)
    {
        ArgumentNullException.ThrowIfNull(text);
        // A lone surrogate becomes U+FFFD here, so the core never sees text that is not UTF-8.
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        using var t = new Lease(term);
        fixed (byte* p = bytes)
        {
            return Sent(send((tt_term*)t.Pointer, p, (nuint)bytes.Length), call);
        }
    }

    // The child not reading or gone is an everyday answer to input, not an error.
    private bool Sent(tt_status status, string call)
    {
        if (status is tt_status.TT_FULL or tt_status.TT_CLOSED)
        {
            return false;
        }
        Check(status, call);
        return true;
    }

    /// <summary>Takes the next event; call it until false after every wakeup.</summary>
    public bool TryPollEvent(out TerminalEvent next)
    {
        var raw = new tt_event { struct_size = (uint)sizeof(tt_event) };
        tt_status status;
        using (var t = new Lease(term))
        {
            status = NativeMethods.tt_term_poll_event((tt_term*)t.Pointer, &raw);
        }
        if (status == tt_status.TT_EMPTY)
        {
            next = default;
            return false;
        }
        Check(status, "tt_term_poll_event");
        uint? exitCode = raw.exit_code == NativeMethods.TT_EXIT_CODE_UNKNOWN ? null : raw.exit_code;
        next = new TerminalEvent((EventKind)raw.kind, exitCode, raw.signaled != 0);
        return true;
    }

    /// <summary>
    /// Brings <see cref="View"/> up to date. False while synchronized output holds
    /// the picture; otherwise the view's damage says what to repaint, possibly
    /// nothing. Spans taken from the previous view are invalid afterwards.
    /// </summary>
    public bool Update()
    {
        var next = new tt_frame_view { struct_size = (uint)sizeof(tt_frame_view) };
        using (var f = new Lease(frame))
        using (var t = new Lease(term))
        {
            Check(NativeMethods.tt_frame_update((tt_frame*)f.Pointer, (tt_term*)t.Pointer, &next), "tt_frame_update");
        }
        view = next;
        return next.updated != 0;
    }

    /// <summary>
    /// The text of <paramref name="rows"/> viewport rows from <paramref name="row"/>,
    /// one line per row, trailing blanks trimmed; rows past the viewport are not read.
    /// </summary>
    public string ReadText(ushort row = 0, ushort rows = ushort.MaxValue)
    {
        byte[] buffer = [];
        // A child can print between asking for the length and reading, so a
        // few tries with some room to spare.
        for (int attempt = 0; attempt < 4; attempt++)
        {
            nuint length = 0;
            tt_status status;
            using (var t = new Lease(term))
            fixed (byte* p = buffer)
            {
                status = NativeMethods.tt_term_read_text((tt_term*)t.Pointer, row, rows, p, (nuint)buffer.Length, &length);
            }
            int needed = checked((int)length);
            if (status == tt_status.TT_OK)
            {
                return Encoding.UTF8.GetString(buffer, 0, needed);
            }
            if (status != tt_status.TT_FULL)
            {
                Check(status, "tt_term_read_text");
            }
            buffer = new byte[checked(needed + needed / 8)];
        }
        throw new ScullException(Status.Full, "tt_term_read_text: the text kept outgrowing the buffer");
    }

    /// <summary>Frees the frame, then the terminal. A second call does nothing.</summary>
    public void Dispose()
    {
        frame.Dispose();
        term.Dispose();
    }

    // Not private so tests can hand the raw handle to the core's test hook.
    internal Lease LeaseTerm() => new(term);

    private void Check(tt_status status, string call)
    {
        if (status == tt_status.TT_OK)
        {
            return;
        }
        if (status is tt_status.TT_POISONED or tt_status.TT_PANIC)
        {
            IsPoisoned = true;
        }
        throw ScullException.From(status, call);
    }

    private static void FreeTerm(nint pointer) => NativeMethods.tt_term_free((tt_term*)pointer);

    private static void FreeFrame(nint pointer) => NativeMethods.tt_frame_free((tt_frame*)pointer);
}

/// <summary>A <c>tt_event.kind</c>; kinds are only appended, so a host skips values it does not know.</summary>
public enum EventKind : uint
{
    Bell = NativeMethods.TT_EVENT_BELL,
    ChildExited = NativeMethods.TT_EVENT_CHILD_EXITED,
}

/// <summary>
/// One polled event. For <see cref="EventKind.ChildExited"/>, the exit code
/// (null when the system did not say) and whether a signal ended the child.
/// </summary>
public readonly record struct TerminalEvent(EventKind Kind, uint? ExitCode, bool Signaled);

/// <summary>What <see cref="Terminal.Spawn"/> runs, and where.</summary>
public sealed record SpawnOptions
{
    /// <summary>Searched on <c>PATH</c>; null runs the user's shell, which ignores <see cref="Arguments"/>.</summary>
    public string? Program { get; init; }

    public IReadOnlyList<string> Arguments { get; init; } = [];

    /// <summary>Null keeps the host's.</summary>
    public string? WorkingDirectory { get; init; }

    /// <summary><c>NAME=value</c> entries added to the host's environment.</summary>
    public IReadOnlyList<string> Environment { get; init; } = [];

    /// <summary>History rows to keep; the core clamps it.</summary>
    public uint Scrollback { get; init; } = 10_000;
}
