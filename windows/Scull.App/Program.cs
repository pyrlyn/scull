using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Scull.Core;

namespace Scull.App;

// Code only, no XAML pages: a window holds one SwapChainPanel and nothing
// the markup compiler would add to, so Main is ours rather than generated.
internal static class Program
{
    [STAThread]
    private static void Main()
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        Application.Start(p =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App();
        });
    }
}

internal sealed partial class App : Application
{
    // Held so a window outlives OnLaunched; it disposes itself when closed. The
    // process ends with the last window.
    private readonly List<Window> windows = [];

    protected override void OnLaunched(LaunchActivatedEventArgs args) => OpenWindow();

    // T20.7.3 HOOK: T20.7.2 assigns the App's opened Config here, and calls
    // Settings?.Refresh(settings) after each poll. Until then Ctrl+, does nothing.
    public Config? Config { get; set; }

    /// <summary>The open settings window, if any.</summary>
    public SettingsWindow? Settings { get; private set; }

    /// <summary>Opens the settings window over <see cref="Config"/>, or brings the open one forward.</summary>
    public void OpenSettings()
    {
        if (Config is not { } config)
        {
            return;
        }
        if (Settings is null)
        {
            Settings = new SettingsWindow(config);
            Settings.Closed += (_, _) => Settings = null;
        }
        Settings.Activate();
    }

    /// <summary>A window of its own: its own swap chain and tabs.</summary>
    public void OpenWindow()
    {
        var window = new TerminalWindow();
        windows.Add(window);
        window.Closed += (_, _) => windows.Remove(window);
        window.Activate();
    }
}
