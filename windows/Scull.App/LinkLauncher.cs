using System.Runtime.InteropServices;
using Windows.Storage;
using Windows.System;

namespace Scull.App;

/// <summary>
/// Opens a link <see cref="Core.EventPolicy.OpenableLink"/> admitted through the system's launcher,
/// never a shell. The launcher refuses executable files itself.
/// </summary>
internal static class LinkLauncher
{
    // Fire and forget: a link that does not open is dropped, as a refused paste is.
    public static async void Open(Uri target)
    {
        try
        {
            if (!target.IsFile)
            {
                await Launcher.LaunchUriAsync(target);
                return;
            }
            string path = target.LocalPath;
            if (Directory.Exists(path))
            {
                await Launcher.LaunchFolderPathAsync(path);
                return;
            }
            await Launcher.LaunchFileAsync(await StorageFile.GetFileFromPathAsync(path));
        }
        catch (Exception e) when (e is COMException or IOException or UnauthorizedAccessException or ArgumentException)
        {
            return;
        }
    }
}
