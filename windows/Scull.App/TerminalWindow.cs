using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Scull.Core;
using Scull.Render;

namespace Scull.App;

/// <summary>
/// One window, one terminal, one swap chain: the shell's surface. Every
/// <c>tt_*</c> call happens on this window's UI thread; the core's wakeup only
/// posts to its dispatcher queue, as TerminalSession.swift does on macOS.
/// </summary>
internal sealed partial class TerminalWindow : Window, IDisposable
{
    private const float FontSize = 14;
    private static readonly string[] FontFamilies = ["Cascadia Mono", "Consolas"];

    // WinUI 3 reports no start or end of a window drag, so a burst of size
    // changes stands in for one: output is held at the first and the grid
    // follows once the size has rested this long.
    private static readonly TimeSpan ResizeSettle = TimeSpan.FromMilliseconds(120);

    private readonly SwapChainPanel panel = new();
    private readonly DispatcherQueueHandler wake;
    private readonly DispatcherQueueTimer resizeTimer;
    private readonly RenderDevice device;
    private Renderer? renderer;
    private SwapChainTarget? target;
    private Terminal? terminal;
    private float scale;
    private bool resizing;

    public TerminalWindow()
    {
        Title = "Scull";
        Content = panel;
        wake = Wake;
        device = CreateDevice();
        resizeTimer = DispatcherQueue.CreateTimer();
        resizeTimer.IsRepeating = false;
        resizeTimer.Interval = ResizeSettle;
        resizeTimer.Tick += (_, _) => FitGrid();
        panel.Loaded += (_, _) => Start();
        panel.SizeChanged += (_, _) => SurfaceChanged();
        // Also a DPI change: the panel's composition scale follows the monitor's.
        panel.CompositionScaleChanged += (_, _) => SurfaceChanged();
        Closed += (_, _) => Dispose();
    }

    private (int Width, int Height) PixelSize =>
        ((int)(panel.ActualWidth * panel.CompositionScaleX), (int)(panel.ActualHeight * panel.CompositionScaleY));

    private static RenderDevice CreateDevice()
    {
        try
        {
            return new RenderDevice();
        }
        catch (COMException)
        {
            // No usable GPU driver (a VM, a remote session): WARP draws the same picture.
            return new RenderDevice(warp: true);
        }
    }

    private void Start()
    {
        if (terminal is not null)
        {
            return;
        }
        Renderer r = CreateRenderer();
        (int width, int height) = PixelSize;
        target = new SwapChainTarget(device, width, height);
        target.Resize(target.Width, target.Height, panel.CompositionScaleX, panel.CompositionScaleY);
        SwapChainPanelNative.SetSwapChain(panel, target.NativeSwapChain);
        (ushort cols, ushort rows) = Grid(r.Metrics, width, height);
        DispatcherQueue queue = DispatcherQueue;
        DispatcherQueueHandler handler = wake;
        var options = new SpawnOptions
        {
            WorkingDirectory = System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile),
            Environment = ["TERM=xterm-256color", "COLORTERM=truecolor"],
        };
        try
        {
            // Runs on a core thread: post and return, the work happens in Wake.
            terminal = Terminal.Spawn(cols, rows, () => queue.TryEnqueue(handler), options);
        }
        catch (ScullException e)
        {
            ShowNotice($"The shell did not start: {e.Message}");
            return;
        }
        Draw(force: true);
    }

    private Renderer CreateRenderer()
    {
        renderer?.Dispose();
        renderer = null;
        scale = panel.CompositionScaleX;
        float pixels = FontSize * Math.Max(scale, 0.5f);
        foreach (string family in FontFamilies)
        {
            try
            {
                return renderer = new Renderer(device, family, pixels);
            }
            catch (ArgumentException) when (family != FontFamilies[^1])
            {
                // Not installed (Cascadia ships with Windows 11 and Terminal); try the next.
            }
        }
        throw new InvalidOperationException("unreachable: the last family rethrows");
    }

    private static (ushort Cols, ushort Rows) Grid(CellMetrics metrics, int width, int height) =>
        ((ushort)Math.Clamp(width / metrics.Width, 1, ushort.MaxValue), (ushort)Math.Clamp(height / metrics.Height, 1, ushort.MaxValue));

    private void Wake()
    {
        if (terminal is null)
        {
            return;
        }
        Guard(() =>
        {
            while (terminal.TryPollEvent(out TerminalEvent next))
            {
                if (next.Kind == EventKind.ChildExited)
                {
                    Close();
                    return;
                }
            }
            Draw(force: false);
        });
    }

    /// <summary>Updates the frame, folds its damage in and draws when it changed or <paramref name="force"/>.</summary>
    private void Draw(bool force)
    {
        if (terminal is null || renderer is null || target is null)
        {
            return;
        }
        bool changed = terminal.Update();
        renderer.Absorb(terminal.View);
        if (changed || force)
        {
            renderer.Render(terminal.View, target);
            target.Present();
        }
    }

    private void SurfaceChanged()
    {
        if (terminal is null || target is null)
        {
            return;
        }
        Guard(() =>
        {
            if (!resizing)
            {
                terminal.ResizeBegin();
                resizing = true;
            }
            if (panel.CompositionScaleX != scale)
            {
                CreateRenderer();
            }
            // The buffers follow at once, so the last frame stays sharp and
            // unstretched while the grid waits for the size to settle.
            (int width, int height) = PixelSize;
            target.Resize(width, height, panel.CompositionScaleX, panel.CompositionScaleY);
            Draw(force: true);
            resizeTimer.Stop();
            resizeTimer.Start();
        });
    }

    private void FitGrid()
    {
        if (terminal is null || renderer is null || target is null)
        {
            return;
        }
        resizing = false;
        Guard(() =>
        {
            (ushort cols, ushort rows) = Grid(renderer.Metrics, target.Width, target.Height);
            // Always sent: it is also what releases the output held since ResizeBegin.
            terminal.Resize(cols, rows, (ushort)Math.Min(cols * renderer.Metrics.Width, ushort.MaxValue),
                (ushort)Math.Min(rows * renderer.Metrics.Height, ushort.MaxValue));
            Draw(force: true);
        });
    }

    /// <summary>A poisoned terminal shows a notice in place of its stale grid; nothing else is left to do with it.</summary>
    private void Guard(Action action)
    {
        try
        {
            action();
        }
        catch (TerminalPoisonedException)
        {
            ShowNotice("This terminal crashed. Close the window to dismiss it.");
        }
    }

    private void ShowNotice(string text) =>
        Content = new TextBlock { Text = text, Margin = new Thickness(FontSize), TextWrapping = TextWrapping.Wrap };

    public void Dispose()
    {
        resizeTimer.Stop();
        // Freeing the terminal joins its threads, so no wakeup follows; one
        // already queued finds it null.
        Terminal? t = terminal;
        terminal = null;
        t?.Dispose();
        renderer?.Dispose();
        target?.Dispose();
        device.Dispose();
    }
}
