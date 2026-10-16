using Windows.Win32;
using Windows.Win32.UI.WindowsAndMessaging;

namespace Scull.App;

/// <summary>What the system says about input, read when it is needed so a change in Control Panel applies at once.</summary>
internal static unsafe class SystemInput
{
    // Windows' own default for SPI_GETWHEELSCROLLLINES.
    private const uint DefaultWheelLines = 3;

    /// <summary>The system's default alert sound, as <c>NSSound.beep()</c> on macOS.</summary>
    public static void Beep() => PInvoke.MessageBeep(MESSAGEBOX_STYLE.MB_OK);

    /// <summary>Lines per wheel notch, or <see cref="Scull.Core.WheelScroll.Page"/> when the user chose a screenful.</summary>
    public static uint WheelLines()
    {
        uint lines = 0;
        return PInvoke.SystemParametersInfo(SYSTEM_PARAMETERS_INFO_ACTION.SPI_GETWHEELSCROLLLINES, 0, &lines, 0) ? lines : DefaultWheelLines;
    }
}
