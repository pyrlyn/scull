using System.Text;
using Scull.Native;

namespace Scull.Core;

/// <summary>What a click on a link may reach; <see cref="LinkTarget.Of"/> asks the core, which decides it for both shells.</summary>
public enum LinkKind : uint
{
    Refused = NativeMethods.TT_LINK_REFUSED,
    /// <summary>An http or https page: open it.</summary>
    Web = NativeMethods.TT_LINK_WEB,
    /// <summary>A mailto address: open it.</summary>
    Mail = NativeMethods.TT_LINK_MAIL,
    /// <summary>A local directory: open it in the file manager.</summary>
    Directory = NativeMethods.TT_LINK_DIRECTORY,
    /// <summary>Any other local file: reveal it in the file manager, never open or run it.</summary>
    File = NativeMethods.TT_LINK_FILE,
}

/// <summary>A link's kind and what to hand the system: the URI, or a local path.</summary>
public readonly record struct LinkTarget(LinkKind Kind, string Target)
{
    /// <summary>
    /// Sorts the OSC 8 <paramref name="uri"/> a click landed on. <paramref name="machine"/> is this
    /// computer's name, which a file URL may give as its host. The URI is the program's text, so
    /// only what this admits may reach the launcher.
    /// </summary>
    public static unsafe LinkTarget Of(string uri, string machine)
    {
        ArgumentNullException.ThrowIfNull(uri);
        ArgumentNullException.ThrowIfNull(machine);
        byte[] text = Encoding.UTF8.GetBytes(uri), name = Encoding.UTF8.GetBytes(machine);
        byte[] buffer = new byte[256];
        // The answer depends only on the text and the file system, so one retry with the length it reported is enough.
        for (int attempt = 0; attempt < 2; attempt++)
        {
            uint kind = 0;
            nuint length = 0;
            tt_status status;
            fixed (byte* t = text, n = name, b = buffer)
            {
                status = NativeMethods.tt_link_target(
                    new tt_str { ptr = t, len = (nuint)text.Length }, new tt_str { ptr = n, len = (nuint)name.Length },
                    &kind, b, (nuint)buffer.Length, &length);
            }
            if (status == tt_status.TT_OK)
            {
                return new LinkTarget((LinkKind)kind, Encoding.UTF8.GetString(buffer, 0, checked((int)length)));
            }
            if (status != tt_status.TT_FULL)
            {
                break;
            }
            buffer = new byte[checked((int)length)];
        }
        return new LinkTarget(LinkKind.Refused, "");
    }
}
