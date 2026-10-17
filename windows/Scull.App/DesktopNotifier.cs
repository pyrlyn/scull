using System.Diagnostics;
using System.Globalization;
using Microsoft.UI.Dispatching;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace Scull.App;

/// <summary>
/// Shows the notifications programs ask for (OSC 9, OSC 777) as system toasts. A notifier that
/// cannot register warns once and stays silent for the session; a toast that fails to show is
/// warned about and the next one tries again. Nothing here may stop the terminal, whatever the
/// notification APIs throw, so the catches are broad on purpose. The policy (when, how often, what
/// text) is <see cref="Scull.Core.NotificationPolicy"/>; this only posts. Every call is on the UI thread.
/// </summary>
internal sealed class DesktopNotifier : IDisposable
{
    // Toasts whose click can still be answered; an older one is forgotten, and its click does nothing.
    private const int Remembered = 32;

    private readonly DispatcherQueue queue = DispatcherQueue.GetForCurrentThread();
    private readonly Dictionary<uint, Action> clicks = [];
    private readonly Queue<uint> order = new();
    private uint next;
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
        catch (Exception e)
        {
            Warn($"registering failed: {e.Message}");
        }
    }

    /// <summary>Shows a toast; <paramref name="onClick"/> runs on the UI thread when it is clicked while the app runs.</summary>
    public void Show(string title, string body, Action onClick)
    {
        if (!working)
        {
            return;
        }
        try
        {
            uint id = ++next;
            // AddText escapes XML (AppNotificationBuilder.cpp, EncodeXml, microsoft/WindowsAppSDK main, checked 2026-10-16).
            AppNotification toast = new AppNotificationBuilder()
                .AddArgument("toast", id.ToString(CultureInfo.InvariantCulture))
                .AddText(title)
                .AddText(body)
                .BuildNotification();
            Remember(id, onClick);
            AppNotificationManager.Default.Show(toast);
        }
        catch (Exception e)
        {
            Warn($"showing failed: {e.Message}");
        }
    }

    private void Remember(uint id, Action onClick)
    {
        clicks[id] = onClick;
        order.Enqueue(id);
        while (order.Count > Remembered)
        {
            clicks.Remove(order.Dequeue());
        }
    }

    // Raised on a thread of the runtime's own.
    private void OnInvoked(AppNotificationManager sender, AppNotificationActivatedEventArgs args)
    {
        if (!args.Arguments.TryGetValue("toast", out string? text) || !uint.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out uint id))
        {
            return;
        }
        queue.TryEnqueue(() =>
        {
            if (!clicks.TryGetValue(id, out Action? click))
            {
                return;
            }
            try
            {
                click();
            }
            catch (Exception e)
            {
                Warn($"a click failed: {e.Message}");
            }
        });
    }

    public void Dispose()
    {
        if (!working)
        {
            return;
        }
        working = false;
        clicks.Clear();
        try
        {
            AppNotificationManager.Default.NotificationInvoked -= OnInvoked;
            AppNotificationManager.Default.Unregister();
        }
        catch (Exception e)
        {
            Warn($"unregistering failed: {e.Message}");
        }
    }

    private static void Warn(string message) => Debug.WriteLine($"Desktop notifications: {message}");
}
