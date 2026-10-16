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
    private const float FontSize = 14;
    private static readonly string[] FontFamilies = ["Cascadia Mono", "Consolas"];

    private readonly RenderDevice device;
    private readonly Terminal terminal;
    private readonly WindowsPointer pointer = new();
    private readonly WheelSteps wheel = new();
    private readonly WheelSteps wheelAcross = new();
    private Renderer renderer;
    private bool focused;

    public TerminalPane(RenderDevice device, Terminal terminal, float scale, (ushort Cols, ushort Rows) cells)
    {
        (this.device, this.terminal, Cells) = (device, terminal, cells);
        renderer = CreateRenderer(device, scale);
        Pairing = new WindowsKeyPairing(key => Send(t => t.Key(key)), text => Send(t => t.Text(text)));
    }

    public WindowsKeyPairing Pairing { get; }

    /// <summary>Where the pane sits in the swap chain, in pixels.</summary>
    public Region Area { get; set; }

    /// <summary>The grid last sent to the core.</summary>
    public (ushort Cols, ushort Rows) Cells { get; private set; }

    /// <summary>The core hit a bug in this terminal; the window draws a notice over the pane.</summary>
    public bool Poisoned { get; private set; }

    public CellMetrics Metrics => renderer.Metrics;

    private static Renderer CreateRenderer(RenderDevice device, float scale)
    {
        float pixels = FontSize * Math.Max(scale, 0.5f);
        foreach (string family in FontFamilies)
        {
            try
            {
                return new Renderer(device, family, pixels);
            }
            catch (ArgumentException) when (family != FontFamilies[^1])
            {
                // Not installed (Cascadia ships with Windows 11 and Terminal); try the next.
            }
        }
        throw new InvalidOperationException("unreachable: the last family rethrows");
    }

    /// <summary>A new scale changes the cell size, so the renderer is made again.</summary>
    public void Rebuild(float scale)
    {
        renderer.Dispose();
        renderer = CreateRenderer(device, scale);
        renderer.Focused = focused;
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

    /// <summary>True when the child has exited.</summary>
    public bool Exited()
    {
        bool exited = false;
        Send(t =>
        {
            while (t.TryPollEvent(out TerminalEvent next))
            {
                exited |= next.Kind == EventKind.ChildExited;
            }
        });
        return exited;
    }

    /// <summary>Runs a call on the terminal; a poisoned one is marked and left alone from then on.</summary>
    public void Send(Action<Terminal> call)
    {
        if (Poisoned)
        {
            return;
        }
        try
        {
            call(terminal);
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

    /// <summary>A mouse event; <paramref name="x"/> and <paramref name="y"/> are window pixels. Not drawn here: what the program makes of it comes back as output.</summary>
    public void Pointer(PointerPoint point, KeyModifiers held, double x, double y)
    {
        PointerPointProperties p = point.Properties;
        if (pointer.Update((int)p.PointerUpdateKind, p.IsLeftButtonPressed, p.IsMiddleButtonPressed, p.IsRightButtonPressed,
                held, CellAt(x, y)) is MouseInput mouse)
        {
            Send(t => t.Mouse(mouse));
        }
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
                t.ScrollDisplay(steps);
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
