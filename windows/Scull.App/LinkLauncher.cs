using System.Runtime.InteropServices;
using Scull.Core;
using Windows.Storage;
using Windows.System;

namespace Scull.App;

/// <summary>
/// Hands a link <see cref="LinkTarget.Of"/> admitted to the system's launcher, never a shell: a page
/// or address opens, a directory opens in Explorer, and any other file is only selected there, so
/// a click never runs something the terminal's program wrote.
/// </summary>
internal static class LinkLauncher
{
    // Fire and forget: a link that does not open is dropped, as a refused paste is.
    public static async void Open(LinkTarget target)
    {
        try
        {
            switch (target.Kind)
            {
                case LinkKind.Web or LinkKind.Mail:
                    await Launcher.LaunchUriAsync(new Uri(target.Target));
                    break;
                case LinkKind.Directory:
                    await Launcher.LaunchFolderPathAsync(target.Target);
                    break;
                case LinkKind.File when Path.GetDirectoryName(target.Target) is { } folder:
                    var options = new FolderLauncherOptions();
                    options.ItemsToSelect.Add(await StorageFile.GetFileFromPathAsync(target.Target));
                    await Launcher.LaunchFolderPathAsync(folder, options);
                    break;
            }
        }
        catch (Exception e) when (e is COMException or IOException or UnauthorizedAccessException or ArgumentException or UriFormatException)
        {
            return;
        }
    }
}
