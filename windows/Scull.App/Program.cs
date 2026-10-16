using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

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

    /// <summary>A window of its own: its own swap chain and tabs.</summary>
    public void OpenWindow()
    {
        var window = new TerminalWindow();
        windows.Add(window);
        window.Closed += (_, _) => windows.Remove(window);
        window.Activate();
    }
}
