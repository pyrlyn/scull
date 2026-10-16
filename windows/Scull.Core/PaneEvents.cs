namespace Scull.Core;

/// <summary>What one drain of a terminal's events found.</summary>
/// <param name="Exited">The child has ended.</param>
/// <param name="TitleChanged">The window title or the working directory differs from before.</param>
/// <param name="Clipboard">The text an OSC 52 write wants on the clipboard (the last, if several); null for none.</param>
/// <param name="Notifications">The notifications to show, cleaned and within the pane's rate limit; null for none.</param>
public readonly record struct DrainedEvents(bool Exited, bool TitleChanged, string? Clipboard, IReadOnlyList<Notice>? Notifications = null)
{
    public IReadOnlyList<Notice> Notices => Notifications ?? [];
}

/// <summary>
/// What a host keeps from a terminal's events, as <c>TerminalSession.swift</c> does on macOS:
/// the window title (an icon name is not one), the working directory, the clipboard text to write,
/// the notifications to show (cleaned, and limited per pane).
/// A clipboard read is always denied: answering it would hand the user's clipboard to whatever
/// the shell runs, which may be a remote machine's program.
/// </summary>
public sealed class PaneEvents
{
    private readonly NotificationLimiter limiter;

    /// <param name="milliseconds">A monotonic clock for the notification limit; the system's by default.</param>
    public PaneEvents(Func<long>? milliseconds = null) => limiter = new NotificationLimiter(milliseconds);

    /// <summary>The title the program set; null while it has set none, or cleared it.</summary>
    public string? Title { get; private set; }

    /// <summary>The local directory the program last reported with OSC 7; null until it does.</summary>
    public string? WorkingDirectory { get; private set; }

    /// <summary>Takes every event the terminal has waiting; call it after each wakeup.</summary>
    public DrainedEvents Drain(Terminal terminal)
    {
        bool exited = false;
        bool changed = false;
        string? clipboard = null;
        List<Notice>? notices = null;
        while (terminal.TryPollEvent(out TerminalEvent next))
        {
            switch (next.Kind)
            {
                case EventKind.ChildExited:
                    exited = true;
                    break;
                case EventKind.Title when EventPolicy.TitleOf(next) is { } title:
                    string? named = title.Length == 0 ? null : title;
                    changed |= named != Title;
                    Title = named;
                    break;
                case EventKind.WorkingDirectory when EventPolicy.PathOfUrl(next.Text) is { } path:
                    changed |= path != WorkingDirectory;
                    WorkingDirectory = path;
                    break;
                case EventKind.ClipboardWrite:
                    clipboard = EventPolicy.ClipboardText(next) ?? clipboard;
                    break;
                case EventKind.Notification:
                    if (NotificationPolicy.Make(next.HeadingText, next.Text) is { } notice && limiter.Allow())
                    {
                        (notices ??= []).Add(notice);
                    }
                    break;
                case EventKind.ClipboardRead:
                    terminal.ClipboardDeny(next.Id);
                    break;
            }
        }
        return new DrainedEvents(exited, changed, clipboard, notices);
    }
}
