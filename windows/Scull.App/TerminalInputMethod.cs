using System.Drawing;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Scull.Core;
using Windows.Win32;

namespace Scull.App;

/// <summary>
/// Joins the TSF context (<see cref="TsfInput"/>) to the window: the context hears of the
/// keyboard focus, the composing text goes to the focused pane's frame, finished text goes to
/// its child as text, and the candidate window is placed from the pane's cells. If TSF cannot
/// start, the terminal runs without an input method (typing and dead keys still work), as it
/// does when any other extension fails.
/// </summary>
internal sealed class TerminalInputMethod : ITsfClient, IDisposable
{
    private readonly FrameworkElement host;
    private readonly Func<TerminalPane?> focused;
    private readonly Action<Action<TerminalPane>> input;
    private TsfInput? tsf;

    /// <param name="input">Hands a call to the focused pane and draws its effect, as the window does for a key.</param>
    public TerminalInputMethod(nint window, FrameworkElement host, Func<TerminalPane?> focused, Action<Action<TerminalPane>> input)
    {
        (Window, this.host, this.focused, this.input) = (window, host, focused, input);
        try
        {
            tsf = new TsfInput(this);
        }
        catch (Exception exception) when (exception is COMException or InvalidOperationException)
        {
            tsf = null;
            return;
        }
        host.GotFocus += (_, _) => Guarded(static t => t.Focus());
        host.LostFocus += (_, _) => Guarded(static t => t.Unfocus());
    }

    public nint Window { get; }

    /// <summary>A composition is open, so the characters that arrive belong to it and not to a key.</summary>
    public bool Composing => tsf?.Composing ?? false;

    /// <summary>
    /// First thing for a key event: true when the key is the input method's (VK_PROCESSKEY)
    /// and says nothing to the terminal. Otherwise a finished composition is delivered first,
    /// so text the key committed goes out before the key does.
    /// </summary>
    public bool Intercept(int virtualKey)
    {
        if (WindowsKeys.IsProcessKey(virtualKey))
        {
            return true;
        }
        Guarded(static t => t.Flush());
        return false;
    }

    public Rectangle CompositionBox() => Screen(focused() is { } pane ? pane.CompositionBox() : default);

    public Rectangle Viewport()
    {
        double scale = host.XamlRoot?.RasterizationScale ?? 1;
        return Screen(new PixelRect(0, 0, host.ActualWidth * scale, host.ActualHeight * scale));
    }

    public void Preedit(string text, int caret, IReadOnlyList<PreeditClause> clauses) => input(pane => pane.SetPreedit(text, caret, clauses));

    public void Commit(string text) => input(pane => pane.Send(t => t.Text(text)));

    public void Dispose()
    {
        tsf?.Dispose();
        tsf = null;
    }

    // A call into TSF that fails is dropped: the next focus or composition starts afresh.
    private void Guarded(Action<TsfInput> call)
    {
        if (tsf is null)
        {
            return;
        }
        try
        {
            call(tsf);
        }
        catch (COMException)
        {
        }
    }

    // Pixels of the swap chain, which fills the host, to pixels of the screen.
    private Rectangle Screen(PixelRect box)
    {
        double scale = host.XamlRoot?.RasterizationScale ?? 1;
        Windows.Foundation.Point inWindow = host.TransformToVisual(null).TransformPoint(new Windows.Foundation.Point(0, 0));
        var origin = new System.Drawing.Point();
        PInvoke.ClientToScreen((Windows.Win32.Foundation.HWND)Window, ref origin);
        int left = origin.X + (int)Math.Round((inWindow.X * scale) + box.X);
        int top = origin.Y + (int)Math.Round((inWindow.Y * scale) + box.Y);
        return new Rectangle(left, top, (int)Math.Round(box.Width), (int)Math.Round(box.Height));
    }
}
