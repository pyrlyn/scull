using System.Text;
using Scull.Native;

namespace Scull.Core;

/// <summary>
/// What a host makes of the events the program can send. Everything here is untrusted text from
/// whatever runs in the terminal, so each answer is the narrow thing the shell may use.
/// </summary>
public static class EventPolicy
{
    private static readonly UTF8Encoding Strict = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    /// <summary>
    /// The name a title event gives, or null when it is not a title for the window: OSC 1 names
    /// the icon only. Control characters are dropped so a name cannot break a tab header's line.
    /// </summary>
    public static string? TitleOf(TerminalEvent e)
    {
        if (e.Kind != EventKind.Title || e.Detail == NativeMethods.TT_TITLE_ICON)
        {
            return null;
        }
        return string.Concat(e.Text.Where(c => !char.IsControl(c)));
    }

    /// <summary>
    /// The local path an OSC 7 <c>file://</c> URL names, or null for anything else. A URL naming
    /// another machine is refused: its path would be taken for a local one.
    /// </summary>
    public static string? PathOfUrl(string url) => LocalPath(url, machine: null);

    /// <summary>
    /// What a click on an OSC 8 link to <paramref name="uri"/> may open: http and https with a
    /// host, mailto, and a <c>file://</c> URL with no host, <c>localhost</c> or
    /// <paramref name="machine"/> (this computer's name), rebuilt from its local path. Null for
    /// everything else; the result goes to the system's launcher, never to a shell.
    /// </summary>
    public static Uri? OpenableLink(string uri, string machine)
    {
        if (uri.StartsWith("file:", StringComparison.OrdinalIgnoreCase))
        {
            return LocalPath(uri, machine) is { } path ? new Uri(path) : null;
        }
        if (!Uri.TryCreate(uri, UriKind.Absolute, out Uri? parsed))
        {
            return null;
        }
        return parsed.Scheme switch
        {
            "http" or "https" when parsed.Host.Length > 0 => parsed,
            "mailto" when uri.Length > "mailto:".Length => parsed,
            _ => null,
        };
    }

    private static string? LocalPath(string url, string? machine)
    {
        const string scheme = "file://";
        if (!url.StartsWith(scheme, StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }
        // The authority is read by hand: Uri calls "localhost" a UNC host on some systems.
        string rest = url[scheme.Length..];
        int slash = rest.IndexOf('/', StringComparison.Ordinal);
        if (slash < 0)
        {
            return null;
        }
        string host = rest[..slash];
        if (host.Length != 0 && !host.Equals("localhost", StringComparison.OrdinalIgnoreCase)
            && (machine is null || !host.Equals(machine, StringComparison.OrdinalIgnoreCase)))
        {
            return null;
        }
        return Uri.TryCreate(scheme + rest[slash..], UriKind.Absolute, out Uri? uri) && uri.IsFile ? uri.LocalPath : null;
    }

    /// <summary>
    /// The text an OSC 52 write puts on the clipboard, or null when its bytes are not UTF-8 (they
    /// are dropped, not repaired, so the clipboard never holds text the program did not send).
    /// </summary>
    public static string? ClipboardText(TerminalEvent e)
    {
        if (e.Kind != EventKind.ClipboardWrite)
        {
            return null;
        }
        try
        {
            return Strict.GetString(e.Body ?? []);
        }
        catch (DecoderFallbackException)
        {
            return null;
        }
    }
}
