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
    {
        if (!abiChecked)
        {
            Abi.EnsureCompatible();
            abiChecked = true;
        }
        var options = new tt_term_options
        {
            struct_size = (uint)sizeof(tt_term_options),
            abi_version = NativeMethods.TT_ABI_VERSION,
            cols = cols,
            rows = rows,
            scrollback = scrollback,
        };
        tt_term* rawTerm = null;
        tt_status status = NativeMethods.tt_term_new(&options, &rawTerm);
        if (status != tt_status.TT_OK)
        {
            throw ScullException.From(status, "tt_term_new");
        }
        term = new NativeHandle((nint)rawTerm, &FreeTerm);
        tt_frame* rawFrame = NativeMethods.tt_frame_new();
        if (rawFrame == null)
        {
            term.Dispose();
            throw new ScullException(Status.Panic, "tt_frame_new failed inside the core");
        }
        frame = new NativeHandle((nint)rawFrame, &FreeFrame);
    }

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
