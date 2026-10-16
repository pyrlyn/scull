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
    /// Whether the last send (<see cref="Key"/>, <see cref="Text"/>, <see cref="Paste"/>, <see cref="Focus"/>
    /// or <see cref="Mouse"/>) was refused because the child is not reading and its input queue is full,
    /// which a host answers with a beep; false when it went through or the child is gone.
    /// </summary>
    public bool InputFull { get; private set; }

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

    /// <summary>
    /// Shows an input method's composing text at the cursor from the next <see cref="Update"/>;
    /// empty clears it. <paramref name="caret"/> is a UTF-16 index into <paramref name="text"/>.
    /// It never reaches the child: commit text with <see cref="Text"/>.
    /// </summary>
    public void SetPreedit(string text, int caret)
    {
        ArgumentNullException.ThrowIfNull(text);
        // The core wants a byte offset; it moves one inside a character back onto its start.
        nuint at = (nuint)Encoding.UTF8.GetByteCount(text.AsSpan(0, Math.Clamp(caret, 0, text.Length)));
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        using var f = new Lease(frame);
        fixed (byte* p = bytes)
        {
            Check(NativeMethods.tt_frame_preedit((tt_frame*)f.Pointer, p, (nuint)bytes.Length, at), "tt_frame_preedit");
        }
    }

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

    /// <summary>Starts a selection of <paramref name="kind"/> at a viewport cell (clamped), replacing any other; output that rewrites its rows, a resize and a screen switch drop it.</summary>
    public void Select(SelectionKind kind, ushort row, ushort col)
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_select_start((tt_term*)t.Pointer, (uint)kind, row, col), "tt_term_select_start");
    }

    /// <summary>Moves the selection's free end to a viewport cell (clamped); nothing without a selection.</summary>
    public void ExtendSelection(ushort row, ushort col)
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_select_extend((tt_term*)t.Pointer, row, col), "tt_term_select_extend");
    }

    /// <summary>Drops the selection.</summary>
    public void ClearSelection()
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_select_clear((tt_term*)t.Pointer), "tt_term_select_clear");
    }

    /// <summary>Whether there is a selection; the core is asked because output can drop it.</summary>
    public bool HasSelection
    {
        get
        {
            nuint length = 0;
            using var t = new Lease(term);
            tt_status status = NativeMethods.tt_term_selection_text((tt_term*)t.Pointer, null, 0, &length);
            if (status != tt_status.TT_EMPTY)
            {
                Check(status is tt_status.TT_FULL ? tt_status.TT_OK : status, "tt_term_selection_text");
            }
            return status != tt_status.TT_EMPTY;
        }
    }

    /// <summary>The selected text as the core copies it (soft-wrapped rows joined, hard line ends as <c>\n</c>, trailing spaces trimmed); null when nothing is selected.</summary>
    public string? SelectionText()
    {
        byte[] buffer = [];
        // The selection can change between asking for the length and reading, so a few tries with some room.
        for (int attempt = 0; attempt < 4; attempt++)
        {
            nuint length = 0;
            tt_status status;
            using (var t = new Lease(term))
            fixed (byte* p = buffer)
            {
                status = NativeMethods.tt_term_selection_text((tt_term*)t.Pointer, p, (nuint)buffer.Length, &length);
            }
            int needed = checked((int)length);
            if (status == tt_status.TT_EMPTY)
            {
                return null;
            }
            if (status == tt_status.TT_OK)
            {
                return Encoding.UTF8.GetString(buffer, 0, needed);
            }
            if (status != tt_status.TT_FULL)
            {
                Check(status, "tt_term_selection_text");
            }
            buffer = new byte[checked(needed + needed / 8)];
        }
        throw new ScullException(Status.Full, "tt_term_selection_text: the text kept outgrowing the buffer");
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
        InputFull = status == tt_status.TT_FULL;
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
        // Only the event polled last has its text, so it is read before anything else polls.
        byte[] body = raw.text_len == 0 ? [] : ReadEventText(raw.serial, NativeMethods.TT_EVENT_TEXT_BODY);
        byte[] heading = raw.title_len == 0 ? [] : ReadEventText(raw.serial, NativeMethods.TT_EVENT_TEXT_TITLE);
        next = new TerminalEvent((EventKind)raw.kind, exitCode, raw.signaled != 0, raw.id, raw.detail, body, heading);
        return true;
    }

    private byte[] ReadEventText(ulong serial, uint part)
    {
        byte[] buffer = [];
        // The text is fixed once polled, so one retry with the length it reported is enough.
        for (int attempt = 0; attempt < 2; attempt++)
        {
            nuint length = 0;
            tt_status status;
            using (var t = new Lease(term))
            fixed (byte* p = buffer)
            {
                status = NativeMethods.tt_term_event_text((tt_term*)t.Pointer, serial, part, p, (nuint)buffer.Length, &length);
            }
            if (status == tt_status.TT_OK)
            {
                return buffer[..checked((int)length)];
            }
            if (status != tt_status.TT_FULL)
            {
                Check(status, "tt_term_event_text");
            }
            buffer = new byte[checked((int)length)];
        }
        throw new ScullException(Status.Full, "tt_term_event_text: the text outgrew its reported length");
    }

    /// <summary>
    /// Refuses the clipboard read <paramref name="id"/> (<see cref="TerminalEvent.Id"/>), so the program
    /// is not left waiting. False when the child is gone.
    /// </summary>
    public bool ClipboardDeny(ulong id)
    {
        using var t = new Lease(term);
        return Sent(NativeMethods.tt_term_clipboard_deny((tt_term*)t.Pointer, id), "tt_term_clipboard_deny");
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

    /// <summary>
    /// Searches the history and the screen for <paramref name="pattern"/>, literal text,
    /// replacing any earlier search; the frame marks the matches in view. Empty ends the
    /// search. False when the core refuses the pattern (over <see cref="MaxSearchPattern"/>
    /// UTF-8 bytes), which ends the search too.
    /// </summary>
    public bool SearchSet(string pattern, bool ignoreCase)
    {
        ArgumentNullException.ThrowIfNull(pattern);
        byte[] bytes = Encoding.UTF8.GetBytes(pattern);
        uint flags = ignoreCase ? NativeMethods.TT_SEARCH_IGNORE_CASE : 0;
        tt_status status;
        using (var t = new Lease(term))
        fixed (byte* p = bytes)
        {
            status = NativeMethods.tt_term_search_set((tt_term*)t.Pointer, new tt_str { ptr = p, len = (nuint)bytes.Length }, flags);
        }
        if (status == tt_status.TT_INVALID)
        {
            return false;
        }
        Check(status, "tt_term_search_set");
        return true;
    }

    /// <summary>
    /// Steps to the next match (or the previous one) from the current match, or from the
    /// viewport's edge, wrapping at the ends; the viewport scrolls to show it. False with
    /// no search or no match.
    /// </summary>
    public bool SearchStep(bool forward, out SearchMatch found)
    {
        var raw = new tt_match { struct_size = (uint)sizeof(tt_match) };
        tt_status status;
        using (var t = new Lease(term))
        {
            status = NativeMethods.tt_term_search_step((tt_term*)t.Pointer, forward ? (byte)1 : (byte)0, &raw);
        }
        if (status == tt_status.TT_EMPTY)
        {
            found = default;
            return false;
        }
        Check(status, "tt_term_search_step");
        found = new SearchMatch(raw.start_col, raw.end_col, raw.start_line, raw.end_line);
        return true;
    }

    /// <summary>Matches in the whole history, up to <see cref="MaxSearchMatches"/>; 0 without a search. It rescans everything.</summary>
    public int SearchCount()
    {
        nuint count = 0;
        using (var t = new Lease(term))
        {
            Check(NativeMethods.tt_term_search_count((tt_term*)t.Pointer, &count), "tt_term_search_count");
        }
        return (int)count;
    }

    /// <summary>The longest pattern <see cref="SearchSet"/> takes, in UTF-8 bytes.</summary>
    public const int MaxSearchPattern = (int)NativeMethods.TT_MAX_SEARCH_PATTERN;

    /// <summary>The most matches <see cref="SearchCount"/> counts.</summary>
    public const int MaxSearchMatches = (int)NativeMethods.TT_MAX_SEARCH_MATCHES;

    /// <summary>The OSC 8 link id of viewport cell (<paramref name="row"/>, <paramref name="col"/>); null where no link covers it.</summary>
    public uint? LinkAt(ushort row, ushort col)
    {
        uint id = 0;
        tt_status status;
        using (var t = new Lease(term))
        {
            status = NativeMethods.tt_term_link_at((tt_term*)t.Pointer, row, col, &id);
        }
        if (status == tt_status.TT_EMPTY)
        {
            return null;
        }
        Check(status, "tt_term_link_at");
        return id;
    }

    /// <summary>
    /// The URI of link <paramref name="id"/> as the program wrote it: untrusted, so the caller
    /// decides whether it may be opened. Null once the link is gone.
    /// </summary>
    public string? LinkUri(uint id)
    {
        byte[] buffer = new byte[256];
        // The URI is fixed while its id lives, so one retry with the length it reported is enough.
        for (int attempt = 0; attempt < 2; attempt++)
        {
            nuint length = 0;
            tt_status status;
            using (var t = new Lease(term))
            fixed (byte* p = buffer)
            {
                status = NativeMethods.tt_term_link_uri((tt_term*)t.Pointer, id, p, (nuint)buffer.Length, &length);
            }
            switch (status)
            {
                case tt_status.TT_OK:
                    return Encoding.UTF8.GetString(buffer, 0, checked((int)length));
                case tt_status.TT_EMPTY:
                    return null;
                case tt_status.TT_FULL:
                    buffer = new byte[checked((int)length)];
                    break;
                default:
                    Check(status, "tt_term_link_uri");
                    break;
            }
        }
        throw new ScullException(Status.Full, "tt_term_link_uri: the URI outgrew its reported length");
    }

    /// <summary>Underlines link <paramref name="id"/> (<see cref="Cell.IsLink"/>) from the next update; null clears it.</summary>
    public void HoverLink(uint? id)
    {
        using var t = new Lease(term);
        Check(NativeMethods.tt_term_link_hover((tt_term*)t.Pointer, id ?? 0), "tt_term_link_hover");
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
    Title = NativeMethods.TT_EVENT_TITLE,
    WorkingDirectory = NativeMethods.TT_EVENT_WORKING_DIRECTORY,
    ShellMark = NativeMethods.TT_EVENT_SHELL_MARK,
    ClipboardRead = NativeMethods.TT_EVENT_CLIPBOARD_READ,
    ClipboardWrite = NativeMethods.TT_EVENT_CLIPBOARD_WRITE,
    Link = NativeMethods.TT_EVENT_LINK,
    Notification = NativeMethods.TT_EVENT_NOTIFICATION,
}

/// <summary>
/// One polled event. For <see cref="EventKind.ChildExited"/>, the exit code
/// (null when the system did not say) and whether a signal ended the child.
/// <paramref name="Id"/> is the clipboard request or the link, <paramref name="Detail"/> the title
/// kind, shell marker or clipboard selection, and <paramref name="Body"/> the event's raw bytes
/// (empty when it has none): not necessarily UTF-8 for a clipboard write. <paramref name="Heading"/> is a
/// notification's title bytes.
/// </summary>
public readonly record struct TerminalEvent(
    EventKind Kind, uint? ExitCode, bool Signaled, ulong Id = 0, uint Detail = 0, byte[]? Body = null, byte[]? Heading = null)
{
    /// <summary>A notification's title as text; empty for OSC 9, which has none.</summary>
    public string HeadingText => Encoding.UTF8.GetString(Heading ?? []);

    /// <summary>The body as text, bad bytes shown as U+FFFD; for kinds that are names, such as titles.</summary>
    public string Text => Encoding.UTF8.GetString(Body ?? []);
}

/// <summary>
/// A search match, first and last cell inclusive (a wide character's right half
/// included). Lines count from the first line ever output, so they hold while the text scrolls.
/// </summary>
public readonly record struct SearchMatch(ushort StartCol, ushort EndCol, ulong StartLine, ulong EndLine);

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
