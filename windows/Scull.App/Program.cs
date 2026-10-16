using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Scull.App;

// Code only, no XAML pages: the window holds one SwapChainPanel and nothing
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
    // Held so the window outlives OnLaunched; it disposes itself when closed.
    private Window? window;

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        window = new TerminalWindow();
        window.Activate();
    }
}
