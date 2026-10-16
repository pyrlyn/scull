using System.Globalization;
using System.Text;

namespace Scull.Core;

/// <summary>A desktop notification a program asked for, cleaned. <see cref="Title"/> is empty when the program gave none.</summary>
public readonly record struct Notice(string Title, string Body)
{
    /// <summary>The title shown: the program's, else the fallback (the tab's title), else the app's name.</summary>
    public string TitleOr(string? fallback)
    {
        if (Title.Length > 0)
        {
            return Title;
        }
        string named = fallback is null ? "" : NotificationPolicy.Clean(fallback, NotificationPolicy.TitleMax);
        return named.Length > 0 ? named : "Scull";
    }
}

/// <summary>
/// What a host does with OSC 9 and OSC 777 requests. The text comes from whatever runs in the
/// terminal, so it is cut down to something a notification can safely carry. The macOS shell
/// applies the same numbers (<c>NotificationPolicy.swift</c>).
/// </summary>
public static class NotificationPolicy
{
    public const int TitleMax = 64;
    public const int BodyMax = 240;

    /// <summary>A notification, or null when the body is empty: a title alone says nothing.</summary>
    public static Notice? Make(string title, string body)
    {
        string text = Clean(body, BodyMax);
        return text.Length == 0 ? null : new Notice(Clean(title, TitleMax), text);
    }

    /// <summary>
    /// <paramref name="text"/> with tabs and line ends as spaces (runs of spaces as one), other control
    /// and invisible formatting characters dropped (a right-to-left override could reorder what the
    /// reader sees), trimmed, and cut to <paramref name="max"/> whole characters with an ellipsis.
    /// </summary>
    public static string Clean(string text, int max)
    {
        ArgumentNullException.ThrowIfNull(text);
        var kept = new StringBuilder(text.Length);
        foreach (char c in text)
        {
            if (c is '\t' or '\n' or '\r' or ' ')
            {
                if (kept.Length > 0 && kept[^1] != ' ')
                {
                    kept.Append(' ');
                }
            }
            else if (!char.IsControl(c) && !IsInvisible(c))
            {
                kept.Append(c);
            }
        }
        string clean = kept.ToString().TrimEnd();
        var elements = StringInfo.ParseCombiningCharacters(clean);
        return elements.Length <= max ? clean : clean[..elements[max - 1]] + "…";
    }

    // Formatting characters, except the joiners emoji and some scripts need.
    private static bool IsInvisible(char c) =>
        c is not ('‌' or '‍') && CharUnicodeInfo.GetUnicodeCategory(c) == UnicodeCategory.Format;
}

/// <summary>
/// A pane's rate limit: a burst of <see cref="Burst"/>, then one more every <see cref="RefillMilliseconds"/>;
/// a notification over the limit is dropped, so a program cannot flood the desktop.
/// </summary>
public sealed class NotificationLimiter(Func<long>? milliseconds = null)
{
    public const int Burst = 3;
    public const long RefillMilliseconds = 5000;

    private readonly Func<long> clock = milliseconds ?? (() => Environment.TickCount64);
    private double tokens = Burst;
    private long last = (milliseconds ?? (() => Environment.TickCount64))();

    /// <summary>True when a notification may show now.</summary>
    public bool Allow()
    {
        long now = clock();
        tokens = Math.Min(Burst, tokens + ((double)Math.Max(0, now - last) / RefillMilliseconds));
        last = now;
        if (tokens < 1)
        {
            return false;
        }
        tokens--;
        return true;
    }
}
