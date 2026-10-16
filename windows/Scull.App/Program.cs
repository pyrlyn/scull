using System.Diagnostics;
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
    private string openError = "";

    /// <summary>The user's config file, shared by every window; null when it could not be opened, and the defaults stay.</summary>
    public Config? Config { get; private set; }

    /// <summary>The settings in force: the last good values, with any error in the file beside them. Null without a config.</summary>
    public ConfigSettings? Settings => Config?.Settings;

    /// <summary>What is wrong with the config, empty when nothing is.</summary>
    public string ConfigError => Settings?.Error is { Length: > 0 } error ? error : openError;

    /// <summary>Raised on the UI thread when a poll found the settings changed.</summary>
    public event Action<ConfigSettings>? ConfigChanged;

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        OpenConfig();
        OpenWindow();
    }

    private void OpenConfig()
    {
        DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
        try
        {
            // The wakeup runs on a core thread: post and return, the poll happens on this one.
            Config = Config.Open(null, () => queue.TryEnqueue(PollConfig));
        }
        catch (ScullException e)
        {
            // A terminal without settings still works; the windows show why.
            openError = $"The config could not be opened: {e.Message}";
            Debug.WriteLine(openError);
            return;
        }
        PollConfig();
    }

    private void PollConfig()
    {
        if (Config?.TryPoll(out ConfigSettings settings) == true)
        {
            ConfigChanged?.Invoke(settings);
        }
    }

    /// <summary>The open settings window, if any.</summary>
    public SettingsWindow? SettingsDialog { get; private set; }

    /// <summary>
    /// Opens the settings window over <see cref="Config"/>, or brings the open one forward.
    /// Without a config there is nothing to edit; the terminal windows already show why.
    /// </summary>
    public void OpenSettings()
    {
        if (Config is not { } config)
        {
            return;
        }
        if (SettingsDialog is null)
        {
            var dialog = new SettingsWindow(config);
            // The window does not poll the config; the app's poll keeps it current, edits made in the file included.
            Action<ConfigSettings> refresh = dialog.Refresh;
            ConfigChanged += refresh;
            dialog.Closed += (_, _) =>
            {
                ConfigChanged -= refresh;
                SettingsDialog = null;
            };
            SettingsDialog = dialog;
        }
        SettingsDialog.Activate();
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
