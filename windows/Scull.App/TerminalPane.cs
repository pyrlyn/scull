using Microsoft.UI.Input;
using Scull.Core;
using Scull.Render;

namespace Scull.App;

/// <summary>
/// One pane: a terminal drawn by its own <see cref="Renderer"/> into a region of its
/// window's single swap chain, with the per-pane input state. It does not own the
/// terminal (the window's <see cref="PaneSet"/> does, so a close frees it once) and
/// every call happens on the window's UI thread.
/// </summary>
internal sealed class TerminalPane : IDisposable
{
    // PointerUpdateKind: the left button going down and up.
    private const int LeftPressed = 1, LeftReleased = 2;

    private readonly RenderDevice device;
    private readonly Terminal terminal;
    private readonly WindowsPointer pointer = new();
    private readonly WheelSteps wheel = new();
    private readonly WheelSteps wheelAcross = new();
    private readonly ClickCounter clicks = new();
    private SelectionDrag? drag;
    private readonly FontZoom font = new();
    private readonly PaneEvents events = new();
    private Renderer renderer;
    private float scale;
    private bool focused;

    /// <param name="settings">The config's settings, null for the defaults.</param>
    public TerminalPane(RenderDevice device, Terminal terminal, float scale, (ushort Cols, ushort Rows) cells, ConfigSettings? settings)
    {
        (this.device, this.terminal, this.scale, Cells) = (device, terminal, scale, cells);
        if (settings is not null)
        {
            font.Apply(settings);
        }
        renderer = CreateRenderer(settings is null ? new Palette() : new Palette(settings));
        // A release the child cannot take is no one's fault, so it does not beep.
        Pairing = new WindowsKeyPairing(key => Type(t => t.Key(key), key.Action != KeyAction.Release), text => Type(t => t.Text(text)));
    }

    public WindowsKeyPairing Pairing { get; }

    /// <summary>Where the pane sits in the swap chain, in pixels.</summary>
    public Region Area { get; set; }

    /// <summary>The grid last sent to the core.</summary>
    public (ushort Cols, ushort Rows) Cells { get; private set; }

    /// <summary>The core hit a bug in this terminal; the window draws a notice over the pane.</summary>
    public bool Poisoned { get; private set; }

    public CellMetrics Metrics => renderer.Metrics;

    private float Pixels => font.Size * Math.Max(scale, 0.5f);

    private Renderer CreateRenderer(Palette palette)
    {
        Renderer? made = null;
        WithFamily(family => made = new Renderer(device, family, Pixels, palette));
        return made!;
    }

    // The configured family, then the system's monospace fonts; the last one's error stands.
    private void WithFamily(Action<string> use)
    {
        IReadOnlyList<string> families = font.Families;
        foreach (string family in families)
        {
            try
            {
                use(family);
                return;
            }
            catch (ArgumentException) when (family != families[^1])
            {
                // Not installed (Cascadia ships with Windows 11 and Terminal); try the next.
            }
        }
    }

    /// <summary>A new scale changes the cell size, so the renderer is made again.</summary>
    public void Rebuild(float scale)
    {
        this.scale = scale;
        Palette palette = renderer.Palette;
        renderer.Dispose();
        renderer = CreateRenderer(palette);
        renderer.Focused = focused;
    }

    /// <summary>Takes new settings: colours at once, a new font only when the file changed it. The window refits the grid after.</summary>
    public void Configure(ConfigSettings settings)
    {
        renderer.Palette = new Palette(settings);
        if (font.Apply(settings))
        {
            WithFamily(family => renderer.SetFont(family, Pixels));
        }
    }

    /// <summary>A font size action; true when the cells changed size and the grid needs refitting.</summary>
    public bool Zoom(ConfigAction action)
    {
        if (!font.Run(action))
        {
            return false;
        }
        WithFamily(family => renderer.SetFont(family, Pixels));
        return true;
    }

    /// <summary>Takes the core's changes into the renderer; true when the picture changed.</summary>
    public bool Update()
    {
        if (Poisoned)
        {
            return false;
        }
        try
        {
            bool changed = terminal.Update();
            renderer.Absorb(terminal.View);
            return changed;
        }
        catch (TerminalPoisonedException)
        {
            Poisoned = true;
            return true;
        }
    }

    public void Paint(RenderTarget target)
    {
        if (!Poisoned)
        {
            renderer.Render(terminal.View, target, Area);
        }
    }

    /// <summary>The pane's text and geometry as of the last update, for UI Automation; null once poisoned.</summary>
    public TextSnapshot? Snapshot()
    {
        if (Poisoned)
        {
            return null;
        }
        try
        {
            return TextSnapshot.Capture(terminal, new CellGeometry(Metrics.Width, Metrics.Height, Area.X, Area.Y));
        }
        catch (TerminalPoisonedException)
        {
            Poisoned = true;
            return null;
        }
    }

    /// <summary>An input method's composing text, shown at the cursor from the next update; empty clears it.</summary>
    public void SetPreedit(string text, int caret) => Send(t => t.SetPreedit(text, caret));

    /// <summary>Where the input method's candidate window goes, in the swap chain's pixels.</summary>
    public PixelRect CompositionBox()
    {
        if (Poisoned)
        {
            return default;
        }
        FrameView view = terminal.View;
        return new CellGeometry(Metrics.Width, Metrics.Height, Area.X, Area.Y).CompositionBox(view.Preedit, view.Cursor);
    }

    /// <summary>The title the program set, null for none.</summary>
    public string? Title => events.Title;

    /// <summary>The directory the program last reported; kept only, as the macOS shell does.</summary>
    public string? WorkingDirectory => events.WorkingDirectory;

    /// <summary>Takes the terminal's waiting events: the child's exit, a new title, text for the clipboard.</summary>
    public DrainedEvents Drain()
    {
        DrainedEvents drained = default;
        Send(t => drained = events.Drain(t));
        return drained;
    }

    /// <summary>Runs a call on the terminal; a poisoned one is marked and left alone from then on.</summary>
    public void Send(Action<Terminal> call) => Type(t =>
    {
        call(t);
        return true;
    }, audible: false);

    /// <summary>
    /// Sends typed input, or a paste, and beeps when the child could not take it because it is not
    /// reading, as <c>TerminalView.swift</c> does on <c>TT_FULL</c>. Poisoning is handled as in <see cref="Send"/>.
    /// </summary>
    public void Type(Func<Terminal, bool> send, bool audible = true)
    {
        if (Poisoned)
        {
            return;
        }
        try
        {
            if (!send(terminal) && audible && terminal.InputFull)
            {
                SystemInput.Beep();
            }
        }
        catch (TerminalPoisonedException)
        {
            Poisoned = true;
        }
    }

    public void BeginResize() => Send(t => t.ResizeBegin());

    /// <summary>
    /// Sends the grid that fits <see cref="Area"/>. Always sent when <paramref name="force"/> is set:
    /// it is also what releases the output held since <see cref="BeginResize"/>.
    /// </summary>
    public void Fit(bool force)
    {
        (ushort cols, ushort rows) = Grid(renderer.Metrics, Area.Width, Area.Height);
        if (!force && (cols, rows) == Cells)
        {
            return;
        }
        Cells = (cols, rows);
        Send(t => t.Resize(cols, rows, (ushort)Math.Min(cols * renderer.Metrics.Width, ushort.MaxValue),
            (ushort)Math.Min(rows * renderer.Metrics.Height, ushort.MaxValue)));
    }

    public static (ushort Cols, ushort Rows) Grid(CellMetrics metrics, int width, int height) =>
        ((ushort)Math.Clamp(width / metrics.Width, 1, ushort.MaxValue), (ushort)Math.Clamp(height / metrics.Height, 1, ushort.MaxValue));

    /// <summary>The program hears of focus only when this pane has the keyboard in the active window.</summary>
    public void SetFocus(bool now)
    {
        if (now == focused)
        {
            return;
        }
        focused = now;
        renderer.Focused = now;
        Send(t => t.Focus(now));
    }

    /// <summary>
    /// A mouse event; <paramref name="x"/> and <paramref name="y"/> are window pixels. True when the
    /// selection changed and wants drawing; what the program makes of a mouse event comes back as output.
    /// </summary>
    public bool Pointer(PointerPoint point, KeyModifiers held, double x, double y)
    {
        PointerPointProperties p = point.Properties;
        int kind = (int)p.PointerUpdateKind;
        var at = CellAt(x, y);
        MouseInput? mouse = pointer.Update(kind, p.IsLeftButtonPressed, p.IsMiddleButtonPressed, p.IsRightButtonPressed, held, at);
        if (kind == LeftPressed)
        {
            drag = null;
            // The program sees the press first, unless Shift asks for the host's selection.
            if (mouse is MouseInput press && !SelectionGesture.ForcesSelection(held) && Taken(press))
            {
                return false;
            }
            return BeginSelection(point, held, x, y, at);
        }
        if (drag is { } selecting && kind != LeftReleased)
        {
            // Motion, or another button: the drag owns the left button, so the program hears neither of its motion.
            if (kind is >= 1 and <= 10)
            {
                SendMouse(mouse);
                return false;
            }
            return Extend(selecting, x, y, at);
        }
        if (drag is { } ended)
        {
            drag = null;
            if (ended.ClearsOnRelease)
            {
                Send(t => t.ClearSelection());
                return true;
            }
            return false;
        }
        SendMouse(mouse);
        return false;
    }

    /// <summary>The selected text, or null when nothing is selected.</summary>
    public string? SelectionText()
    {
        string? text = null;
        Send(t => text = t.SelectionText());
        return text;
    }

    private bool Taken(MouseInput mouse)
    {
        bool taken = false;
        Send(t => taken = t.Mouse(mouse));
        return taken;
    }

    private void SendMouse(MouseInput? mouse)
    {
        if (mouse is MouseInput m)
        {
            Send(t => t.Mouse(m));
        }
    }

    private bool BeginSelection(PointerPoint point, KeyModifiers held, double x, double y, (uint Col, uint Row, uint XPx, uint YPx) at)
    {
        (long interval, double slop) = SystemInput.DoubleClick();
        // The timestamp is in microseconds.
        int clicks = this.clicks.Press((long)(point.Timestamp / 1000), x, y, interval, slop);
        bool has = false;
        Send(t => has = t.HasSelection);
        SelectionPress press = SelectionGesture.Press(clicks, held, has);
        ushort row = (ushort)at.Row, col = (ushort)at.Col;
        Send(t =>
        {
            if (press.Extend)
            {
                t.ExtendSelection(row, col);
            }
            else
            {
                t.Select(press.Kind, row, col);
            }
        });
        drag = new SelectionDrag(at.Col, at.Row, press.ClearsOnRelease);
        return true;
    }

    private bool Extend(SelectionDrag selecting, double x, double y, (uint Col, uint Row, uint XPx, uint YPx) at)
    {
        // Past an edge the history scrolls a row per motion event.
        int scroll = SelectionDrag.EdgeScroll(y - Area.Y, Area.Height);
        if (!selecting.Move(at.Col, at.Row, scroll != 0))
        {
            return false;
        }
        Send(t =>
        {
            if (scroll != 0)
            {
                t.ScrollDisplay(scroll);
            }
            t.ExtendSelection((ushort)at.Row, (ushort)at.Col);
        });
        return true;
    }

    /// <summary>The pointer capture was lost or cancelled: releases the buttons still held, so the program sees none stuck.</summary>
    public void LosePointer(KeyModifiers held)
    {
        foreach (MouseInput release in pointer.Lose(held))
        {
            // The drag took the left press, so the program never saw it.
            if (drag is null || release.Button != MouseButton.Left)
            {
                Send(t => t.Mouse(release));
            }
        }
        drag = null;
    }

    /// <summary>A wheel turn; true when the view scrolled and wants drawing.</summary>
    public bool Wheel(PointerPoint point, KeyModifiers held, double x, double y)
    {
        bool across = point.Properties.IsHorizontalMouseWheel;
        int steps = (across ? wheelAcross : wheel).Add(point.Properties.MouseWheelDelta);
        if (steps == 0)
        {
            return false;
        }
        MouseInput step = pointer.Wheel(steps, across, held, CellAt(x, y));
        bool scrolled = false;
        Send(t =>
        {
            if (t.Mouse(step))
            {
                for (int i = 1; i < Math.Abs(steps); i++)
                {
                    t.Mouse(step);
                }
            }
            else if (!across)
            {
                // History scrolling sends no wakeup, so the view refreshes here.
                t.ScrollDisplay(WheelScroll.Rows(steps, SystemInput.WheelLines(), Cells.Rows));
                scrolled = true;
            }
        });
        return scrolled;
    }

    private (uint Col, uint Row, uint XPx, uint YPx) CellAt(double x, double y)
    {
        FrameView view = terminal.View;
        return PointerCells.At(x - Area.X, y - Area.Y, renderer.Metrics.Width, renderer.Metrics.Height, view.Cols, view.Rows);
    }

    public void Dispose() => renderer.Dispose();
}
