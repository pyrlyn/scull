using System.Runtime.InteropServices;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Scull.Core;
using Scull.Render;

namespace Scull.App;

/// <summary>
/// One window: a tab strip over one swap chain. Each tab is a <see cref="PaneSet"/>, and
/// the selected tab's panes are viewports of that one swap chain, each drawn by its own
/// <see cref="TerminalPane"/>. Every <c>tt_*</c> call happens on this window's UI thread;
/// the core's wakeup only posts to its dispatcher queue, as TerminalSession.swift does on macOS.
/// </summary>
internal sealed partial class TerminalWindow : Window, IDisposable
{
    // What shows between panes; the panes' own background fills the rest.
    private const uint DividerColor = 0x3C3C3C;

    // WinUI 3 reports no start or end of a window drag, so a burst of size
    // changes stands in for one: output is held at the first and the grid
    // follows once the size has rested this long.
    private static readonly TimeSpan ResizeSettle = TimeSpan.FromMilliseconds(120);

    private static readonly SpawnOptions Spawn = new()
    {
        WorkingDirectory = System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile),
        Environment = ["TERM=xterm-256color", "COLORTERM=truecolor"],
    };

    private sealed class Tab(TabViewItem item)
    {
        public TabViewItem Item { get; } = item;
        public PaneSet Panes { get; set; } = null!;
        public Dictionary<int, TerminalPane> Views { get; } = [];
        public IReadOnlyList<PaneViewport> Layout { get; set; } = [];
        public bool Closed { get; set; }

        public TerminalPane Focused => Views[Panes.Tree.Focused];
    }

    private readonly App app = (App)Application.Current;
    private readonly InfoBar configError = new() { Severity = InfoBarSeverity.Error, Title = "Config" };
    private readonly TabView tabView;
    private readonly SwapChainPanel panel = new();
    private readonly Canvas notices = new() { IsHitTestVisible = false };
    private readonly TerminalHost host;
    private readonly TerminalInputMethod ime;
    private readonly List<Tab> tabs = [];
    private readonly DispatcherQueueTimer resizeTimer;
    private readonly RenderDevice device;
    private SwapChainTarget? target;
    private Tab? current;
    private TerminalPane? pressed;
    private float scale;
    private int tabCount;
    private bool resizing;
    private bool active;
    private bool keyboard;

    public TerminalWindow()
    {
        Title = "Scull";
        tabView = new TabView { VerticalAlignment = VerticalAlignment.Top };
        tabView.AddTabButtonClick += (_, _) => Run(PaneCommand.NewTab);
        tabView.TabCloseRequested += (_, e) => CloseTab((Tab)e.Tab.Tag);
        tabView.SelectionChanged += (_, _) =>
        {
            if (tabView.SelectedItem is TabViewItem { Tag: Tab tab })
            {
                Select(tab);
            }
        };
        host = new TerminalHost(FocusedSnapshot)
        {
            // The panel is a Grid, which takes no focus; this control does, so keys come here.
            IsTabStop = true,
            UseSystemFocusVisuals = false,
            // A swap chain is nothing XAML can hit-test; a transparent background is.
            Content = new Grid { Background = new SolidColorBrush(Colors.Transparent), Children = { panel, notices } },
        };
        Grid.SetRow(configError, 1);
        Grid.SetRow(host, 2);
        // Over the terminal but outside its host, so typing in the field never reaches the program.
        Grid.SetRow(findBar, 2);
        Content = new Grid
        {
            RowDefinitions = { new RowDefinition { Height = GridLength.Auto }, new RowDefinition { Height = GridLength.Auto }, new RowDefinition() },
            Children = { tabView, configError, host, findBar },
        };
        ShowConfigError();
        app.ConfigChanged += OnConfig;
        WireInput();
        WireFind();
        ime = new TerminalInputMethod(WinRT.Interop.WindowNative.GetWindowHandle(this), host, FocusedPane, Input);
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

    private void WireInput()
    {
        host.KeyDown += OnKeyDown;
        host.KeyUp += (_, e) => Input(pane => e.Handled = !ime.Intercept((int)e.Key) && pane.Pairing.KeyUp((int)e.Key, e.KeyStatus.ScanCode, e.KeyStatus.IsExtendedKey, Held()));
        host.CharacterReceived += (_, e) =>
        {
            if (!ime.Composing)
            {
                Input(pane => pane.Pairing.Character(e.Character));
            }
            e.Handled = true;
        };
        host.PointerPressed += (_, e) =>
        {
            host.Focus(FocusState.Pointer);
            // Drags keep reporting past the window's edge until the last button goes up.
            host.CapturePointer(e.Pointer);
            PointerPoint point = e.GetCurrentPoint(panel);
            if (current is { } tab && PaneUnder(point, out int id) is { } under)
            {
                pressed = under;
                tab.Panes.Tree.Focus(id);
                UpdateFocus();
            }
            OnPointer(e);
        };
        host.PointerMoved += (_, e) => OnPointer(e);
        host.PointerReleased += (_, e) =>
        {
            OnPointer(e);
            PointerPointProperties p = e.GetCurrentPoint(panel).Properties;
            if (!p.IsLeftButtonPressed && !p.IsMiddleButtonPressed && !p.IsRightButtonPressed)
            {
                pressed = null;
            }
            host.ReleasePointerCapture(e.Pointer);
        };
        host.PointerWheelChanged += OnWheel;
        host.GotFocus += (_, _) =>
        {
            keyboard = true;
            UpdateFocus();
        };
        host.LostFocus += (_, _) =>
        {
            keyboard = false;
            UpdateFocus();
        };
        Activated += (_, e) =>
        {
            active = e.WindowActivationState != WindowActivationState.Deactivated;
            UpdateFocus();
        };
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
        if (target is not null)
        {
            return;
        }
        host.Focus(FocusState.Programmatic);
        scale = panel.CompositionScaleX;
        (int width, int height) = PixelSize;
        target = new SwapChainTarget(device, width, height);
        target.Resize(target.Width, target.Height, panel.CompositionScaleX, panel.CompositionScaleY);
        SwapChainPanelNative.SetSwapChain(panel, target.NativeSwapChain);
        try
        {
            AddTab();
        }
        catch (ScullException e)
        {
            Content = new TextBlock { Text = $"The shell did not start: {e.Message}", Margin = new Thickness(14), TextWrapping = TextWrapping.Wrap };
        }
    }

    private void AddTab()
    {
        var tab = new Tab(new TabViewItem { Header = $"Terminal {++tabCount}" });
        tab.Item.Tag = tab;
        // The first pane is made inside the constructor, so a shell that will not start leaves no tab behind.
        tab.Panes = new PaneSet(id => SpawnPane(tab, id));
        tabs.Add(tab);
        tabView.TabItems.Add(tab.Item);
        tabView.SelectedItem = tab.Item;
    }

    // A new pane starts with its neighbour's grid; the layout that follows resizes both to fit.
    private Terminal SpawnPane(Tab tab, int id)
    {
        (ushort cols, ushort rows) = tab.Panes is { } set ? tab.Views[set.Tree.Focused].Cells : ((ushort)80, (ushort)24);
        DispatcherQueue queue = DispatcherQueue;
        DispatcherQueueHandler wake = () => Wake(tab, id);
        // Runs on a core thread: post and return, the work happens in Wake.
        // Scrollback is fixed at spawn, so a change reaches terminals opened after it.
        SpawnOptions spawn = app.Settings is { } settings ? Spawn with { Scrollback = settings.Scrollback } : Spawn;
        Terminal terminal = Terminal.Spawn(cols, rows, () => queue.TryEnqueue(wake), spawn);
        try
        {
            tab.Views[id] = new TerminalPane(device, terminal, scale, (cols, rows), app.Settings);
        }
        catch
        {
            terminal.Dispose();
            throw;
        }
        return terminal;
    }

    private void Select(Tab tab)
    {
        current = tab;
        host.Focus(FocusState.Programmatic);
        Relayout(tab, force: false);
        UpdateFocus();
    }

    private void CloseTab(Tab tab)
    {
        if (tab.Closed)
        {
            return;
        }
        tab.Closed = true;
        tabs.Remove(tab);
        if (current == tab)
        {
            current = null;
        }
        pressed = null;
        // Removing the item selects a neighbour, which draws it.
        tabView.TabItems.Remove(tab.Item);
        Free(tab);
        if (tabs.Count == 0)
        {
            Close();
        }
    }

    // Freeing the terminals joins their threads, so no wakeup follows; one already queued finds the tab closed.
    private static void Free(Tab tab)
    {
        tab.Panes.Dispose();
        foreach (TerminalPane pane in tab.Views.Values)
        {
            pane.Dispose();
        }
        tab.Views.Clear();
    }

    private void ClosePane(Tab tab, int id)
    {
        if (!tab.Panes.Tree.Panes.Contains(id))
        {
            return;
        }
        if (tab.Panes.Tree.Panes.Count == 1)
        {
            CloseTab(tab);
            return;
        }
        tab.Panes.Close(id);
        tab.Views.Remove(id, out TerminalPane? gone);
        gone?.Dispose();
        pressed = null;
        if (tab == current)
        {
            Relayout(tab, force: false);
            UpdateFocus();
        }
    }

    private void Run(PaneCommand command)
    {
        if (current is not { } tab)
        {
            return;
        }
        try
        {
            switch (command)
            {
                case PaneCommand.NewTab:
                    AddTab();
                    break;
                case PaneCommand.Find:
                    OpenFind();
                    break;
                case PaneCommand.NewWindow:
                    ((App)Application.Current).OpenWindow();
                    break;
                case PaneCommand.OpenSettings:
                    ((App)Application.Current).OpenSettings();
                    break;
                case PaneCommand.SplitRight:
                case PaneCommand.SplitDown:
                    tab.Panes.Split(command == PaneCommand.SplitRight ? SplitAxis.SideBySide : SplitAxis.Stacked);
                    Relayout(tab, force: false);
                    UpdateFocus();
                    break;
                case PaneCommand.ClosePane:
                    ClosePane(tab, tab.Panes.Tree.Focused);
                    break;
                case PaneCommand.NextPane:
                case PaneCommand.PreviousPane:
                    tab.Panes.Tree.FocusNext(command == PaneCommand.NextPane ? 1 : -1);
                    UpdateFocus();
                    break;
            }
        }
        catch (ScullException e)
        {
            // A shell that would not start; the tab strip has no room for a message.
            Title = $"Scull: {e.Message}";
        }
    }

    private void OnConfig(ConfigSettings settings)
    {
        foreach (TerminalPane pane in tabs.SelectMany(t => t.Views.Values))
        {
            pane.Configure(settings);
        }
        ShowConfigError();
        // The other tabs refit when selected.
        if (current is { } tab)
        {
            Relayout(tab, force: true);
        }
    }

    // The core keeps the last good values when the file breaks, so only the message changes.
    private void ShowConfigError()
    {
        configError.Message = app.ConfigError;
        configError.IsOpen = configError.Message.Length > 0;
    }

    /// <summary>Runs a configured key binding on the focused pane, as TerminalView.swift's <c>runBinding</c> does.</summary>
    private void RunBinding(ConfigAction action)
    {
        if (current is not { } tab)
        {
            return;
        }
        TerminalPane pane = tab.Focused;
        if (action == ConfigAction.Paste)
        {
            PasteClipboard();
        }
        else if (ConfigBindings.ScrollRows(action, pane.Cells.Rows) is int rows)
        {
            // History scrolling sends no wakeup, so the view refreshes here.
            pane.Send(t => t.ScrollDisplay(rows));
            Redraw(force: false);
        }
        else if (pane.Zoom(action))
        {
            Relayout(tab, force: true);
        }
    }

    private void Wake(Tab tab, int id)
    {
        if (tab.Closed || !tab.Views.TryGetValue(id, out TerminalPane? pane))
        {
            return;
        }
        if (pane.Exited())
        {
            ClosePane(tab, id);
        }
        else if (tab == current)
        {
            Redraw(force: false);
        }
        else
        {
            pane.Update();
        }
    }

    /// <summary>
    /// Updates the selected tab's panes and draws them all when one changed or <paramref name="force"/>:
    /// the swap chain's buffers rotate, so a frame is never built on the last one.
    /// </summary>
    private void Redraw(bool force)
    {
        if (target is null || current is not { } tab)
        {
            return;
        }
        FollowFind();
        bool changed = force;
        foreach (TerminalPane pane in tab.Views.Values)
        {
            changed |= pane.Update();
        }
        if (!changed)
        {
            return;
        }
        target.Clear(DividerColor);
        foreach (TerminalPane pane in tab.Views.Values)
        {
            pane.Paint(target);
        }
        target.Present();
        ShowNotices(tab);
        host.TextChanged();
    }

    // What a UI Automation client reads: the pane the typing goes to.
    private TerminalPane? FocusedPane() =>
        current is { } tab && tab.Views.TryGetValue(tab.Panes.Tree.Focused, out TerminalPane? pane) ? pane : null;

    private TextSnapshot? FocusedSnapshot() => FocusedPane()?.Snapshot();

    // A poisoned pane shows a notice in place of its stale grid; the other panes keep running.
    private void ShowNotices(Tab tab)
    {
        if (notices.Children.Count == 0 && !tab.Views.Values.Any(p => p.Poisoned))
        {
            return;
        }
        notices.Children.Clear();
        foreach (TerminalPane pane in tab.Views.Values.Where(p => p.Poisoned))
        {
            Region a = pane.Area;
            var notice = new Border
            {
                Width = a.Width / scale,
                Height = a.Height / scale,
                Background = new SolidColorBrush(Colors.DimGray),
                Child = new TextBlock
                {
                    Text = "This terminal crashed. Close the pane to dismiss it.",
                    Margin = new Thickness(14),
                    TextWrapping = TextWrapping.Wrap,
                    Foreground = new SolidColorBrush(Colors.White),
                },
            };
            Canvas.SetLeft(notice, a.X / scale);
            Canvas.SetTop(notice, a.Y / scale);
            notices.Children.Add(notice);
        }
    }

    // Places the tab's panes on the surface and sends each the grid that fits its region.
    private void Relayout(Tab tab, bool force)
    {
        if (target is null)
        {
            return;
        }
        int divider = Math.Max(1, (int)Math.Round(scale));
        tab.Layout = tab.Panes.Tree.Layout(target.Width, target.Height, divider);
        foreach (PaneViewport v in tab.Layout)
        {
            TerminalPane pane = tab.Views[v.Pane];
            pane.Area = new Region(v.X, v.Y, v.Width, v.Height);
            if (!resizing)
            {
                pane.Fit(force);
            }
        }
        Redraw(force: true);
    }

    private void SurfaceChanged()
    {
        if (target is null || current is not { } tab)
        {
            return;
        }
        if (!resizing)
        {
            foreach (TerminalPane pane in tab.Views.Values)
            {
                pane.BeginResize();
            }
            resizing = true;
        }
        if (panel.CompositionScaleX != scale)
        {
            scale = panel.CompositionScaleX;
            foreach (TerminalPane pane in tabs.SelectMany(t => t.Views.Values))
            {
                pane.Rebuild(scale);
            }
        }
        // The buffers follow at once, so the last frame stays sharp and
        // unstretched while the grids wait for the size to settle.
        (int width, int height) = PixelSize;
        target.Resize(width, height, panel.CompositionScaleX, panel.CompositionScaleY);
        Relayout(tab, force: false);
        resizeTimer.Stop();
        resizeTimer.Start();
    }

    private void FitGrid()
    {
        resizing = false;
        if (current is { } tab)
        {
            // Always sent: it is also what releases the output held since BeginResize.
            Relayout(tab, force: true);
        }
    }

    // KeyRoutedEventArgs carries no modifiers; the keyboard's state as of this event does.
    private static KeyModifiers Held() =>
        WindowsKeys.Held(static key => (int)InputKeyboardSource.GetKeyStateForCurrentThread((Windows.System.VirtualKey)key));

    /// <summary>Hands input to the focused pane, then shows its effect at once: typing returns the view to the screen.</summary>
    private void Input(Action<TerminalPane> send)
    {
        if (current is { } tab)
        {
            send(tab.Focused);
            Redraw(force: false);
        }
    }

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        int key = (int)e.Key;
        if (ime.Intercept(key))
        {
            return;
        }
        (uint scanCode, bool extended) = (e.KeyStatus.ScanCode, e.KeyStatus.IsExtendedKey);
        KeyModifiers held = Held();
        if (PaneCommands.Match(key, held) is { } command)
        {
            e.Handled = true;
            Run(command);
            return;
        }
        uint? mapped = WindowsKeys.Map(key, scanCode, extended);
        if (app.Settings is { } settings && ConfigBindings.Match(settings.Keybinds, mapped, held) is { } action)
        {
            e.Handled = true;
            RunBinding(action);
            return;
        }
        if (WindowsKeys.IsPaste(mapped, held))
        {
            e.Handled = true;
            PasteClipboard();
            return;
        }
        // Handled once sent, so Tab and the arrows do not move XAML's focus.
        Input(pane => e.Handled = pane.Pairing.KeyDown(key, scanCode, extended, e.KeyStatus.WasKeyDown, held));
    }

    private async void PasteClipboard()
    {
        string text;
        try
        {
            DataPackageView content = Clipboard.GetContent();
            if (!content.Contains(StandardDataFormats.Text))
            {
                return;
            }
            text = await content.GetTextAsync();
        }
        catch (COMException)
        {
            // Another program holds the clipboard open; the paste is dropped, as a text box does.
            return;
        }
        Input(pane => pane.Send(t => t.Paste(text)));
    }

    // Positions come in DIPs; panes are laid out in the swap chain's physical pixels.
    private TerminalPane? PaneUnder(PointerPoint point, out int id)
    {
        id = 0;
        if (current is not { } tab || PaneTree.PaneAt(tab.Layout, point.Position.X * panel.CompositionScaleX, point.Position.Y * panel.CompositionScaleY) is not { } found)
        {
            return null;
        }
        id = found;
        return tab.Views[found];
    }

    private void OnPointer(PointerRoutedEventArgs e)
    {
        e.Handled = true;
        PointerPoint point = e.GetCurrentPoint(panel);
        // A drag stays with the pane it started in.
        TerminalPane? pane = pressed ?? PaneUnder(point, out _);
        pane?.Pointer(point, Held(), point.Position.X * panel.CompositionScaleX, point.Position.Y * panel.CompositionScaleY);
    }

    private void OnWheel(object sender, PointerRoutedEventArgs e)
    {
        e.Handled = true;
        PointerPoint point = e.GetCurrentPoint(panel);
        if (PaneUnder(point, out _) is { } pane
            && pane.Wheel(point, Held(), point.Position.X * panel.CompositionScaleX, point.Position.Y * panel.CompositionScaleY))
        {
            Redraw(force: false);
        }
    }

    // The program hears of focus only in the focused pane of the selected tab, with the keyboard in the active window.
    private void UpdateFocus()
    {
        bool window = active && keyboard;
        foreach (Tab tab in tabs)
        {
            foreach ((int id, TerminalPane pane) in tab.Views)
            {
                pane.SetFocus(window && tab == current && id == tab.Panes.Tree.Focused);
            }
        }
        Redraw(force: true);
    }

    public void Dispose()
    {
        app.ConfigChanged -= OnConfig;
        resizeTimer.Stop();
        ime.Dispose();
        foreach (Tab tab in tabs)
        {
            tab.Closed = true;
            Free(tab);
        }
        tabs.Clear();
        current = null;
        target?.Dispose();
        device.Dispose();
    }
}
