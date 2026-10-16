using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace Scull.App;

/// <summary>
/// Shows the notifications programs ask for (OSC 9, OSC 777) as system toasts. A notifier that
/// cannot register, or whose toast is refused, warns once and stays silent from then on: a
/// terminal without notifications is still a terminal. The policy (when, how often, what text)
/// is <see cref="Scull.Core.NotificationPolicy"/>; this only posts. Every call is on the UI thread.
/// </summary>
internal sealed class DesktopNotifier : IDisposable
{
    private readonly DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
    private Action? activate;
    private bool working;

    public DesktopNotifier()
    {
        try
        {
            if (!AppNotificationManager.IsSupported())
            {
                Warn("notifications are not supported here");
                return;
            }
            AppNotificationManager.Default.NotificationInvoked += OnInvoked;
            AppNotificationManager.Default.Register();
            working = true;
        }
        catch (Exception e) when (Expected(e))
        {
            Warn($"registering failed: {e.Message}");
        }
    }

    /// <summary>Shows a toast; <paramref name="onClick"/> runs on the UI thread if it is clicked while the app runs (the latest toast's wins).</summary>
    public void Show(string title, string body, Action onClick)
    {
        if (!working)
        {
            return;
        }
        try
        {
            AppNotification toast = new AppNotificationBuilder().AddText(title).AddText(body).BuildNotification();
            activate = onClick;
            AppNotificationManager.Default.Show(toast);
        }
        catch (Exception e) when (Expected(e))
        {
            working = false;
            Warn($"showing failed: {e.Message}");
        }
    }

    // Raised on a thread of the runtime's own.
    private void OnInvoked(AppNotificationManager sender, AppNotificationActivatedEventArgs args) =>
        queue.TryEnqueue(() => activate?.Invoke());

    public void Dispose()
    {
        if (!working)
        {
            return;
        }
        working = false;
        try
        {
            AppNotificationManager.Default.NotificationInvoked -= OnInvoked;
            AppNotificationManager.Default.Unregister();
        }
        catch (Exception e) when (Expected(e))
        {
            Warn($"unregistering failed: {e.Message}");
        }
    }

    // What the notification APIs throw when the system will not have us: not a bug to crash on.
    private static bool Expected(Exception e) =>
        e is COMException or InvalidOperationException or NotSupportedException or UnauthorizedAccessException or IOException;

    private static void Warn(string message) => Debug.WriteLine($"Desktop notifications off: {message}");
}
