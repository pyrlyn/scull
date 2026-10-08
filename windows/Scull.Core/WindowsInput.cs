namespace Scull.Core;

// Windows input in the core's vocabulary, from the plain numbers WinUI reports
// (VirtualKey, CorePhysicalKeyStatus, MouseWheelDelta), so it is tested on
// every runner without WinUI. Virtual keys:
// https://learn.microsoft.com/windows/win32/inputdev/virtual-key-codes;
// scan codes are PC/AT set 1, as in scull-input's win32.rs.

/// <summary>Windows virtual keys and scan codes to <see cref="KeyCode"/>.</summary>
public static class WindowsKeys
{
    private const KeyModifiers Shortcut = KeyModifiers.Shift | KeyModifiers.Alt | KeyModifiers.Control | KeyModifiers.Super;

    // The main block by position: what each scan code types on the US layout,
    // which is how the core names these keys whatever the layout.
    private static readonly (uint First, string Keys)[] MainBlock =
        [(0x02, "1234567890-="), (0x10, "qwertyuiop[]"), (0x1E, "asdfghjkl;'`"), (0x2B, "\\zxcvbnm,./"), (0x39, " ")];

    // The modifier keys from Left Shift in the core's order, left then right.
    private static readonly KeyModifiers[] SideModifiers =
        [KeyModifiers.Shift, KeyModifiers.Control, KeyModifiers.Alt, KeyModifiers.Super, KeyModifiers.Hyper, KeyModifiers.Meta];

    // CoreVirtualKeyStates bits.
    private const int Down = 1;
    private const int Locked = 2;

    private static readonly (int VirtualKey, int Bit, KeyModifiers Modifier)[] HeldKeys =
    [
        (0x10, Down, KeyModifiers.Shift), (0x11, Down, KeyModifiers.Control), (0x12, Down, KeyModifiers.Alt),
        (0x5B, Down, KeyModifiers.Super), (0x5C, Down, KeyModifiers.Super),
        (0x14, Locked, KeyModifiers.CapsLock), (0x90, Locked, KeyModifiers.NumLock),
    ];

    /// <summary>
    /// The core's key, or null for one it has no name for (an input method's
    /// VK_PROCESSKEY, a key the US layout lacks, such as the ISO key by Left Shift).
    /// </summary>
    public static uint? Map(int virtualKey, uint scanCode, bool extended) => virtualKey switch
    {
        0x1B => KeyCode.Escape,
        0x0D => extended ? KeyCode.KeypadEnter : KeyCode.Enter,
        0x09 => KeyCode.Tab,
        0x08 => KeyCode.Backspace,
        // With Num Lock off the keypad sends the editing block's virtual keys;
        // only the extended bit, set on the dedicated keys, tells them apart.
        0x2D => extended ? KeyCode.Insert : KeyCode.Keypad0 + 26,
        0x2E => extended ? KeyCode.Delete : KeyCode.Keypad0 + 27,
        0x25 => extended ? KeyCode.Left : KeyCode.KeypadLeft,
        0x27 => extended ? KeyCode.Right : KeyCode.KeypadLeft + 1,
        0x26 => extended ? KeyCode.Up : KeyCode.KeypadLeft + 2,
        0x28 => extended ? KeyCode.Down : KeyCode.KeypadLeft + 3,
        0x21 => extended ? KeyCode.PageUp : KeyCode.KeypadLeft + 4,
        0x22 => extended ? KeyCode.PageDown : KeyCode.KeypadLeft + 5,
        0x24 => extended ? KeyCode.Home : KeyCode.KeypadLeft + 6,
        0x23 => extended ? KeyCode.End : KeyCode.KeypadLeft + 7,
        0x0C => KeyCode.Keypad0 + 28,
        0x14 => KeyCode.CapsLock,
        0x91 => KeyCode.ScrollLock,
        0x90 => KeyCode.NumLock,
        0x2C => KeyCode.PrintScreen,
        0x13 => KeyCode.Pause,
        0x5D => KeyCode.Menu,
        >= 0x70 and <= 0x87 => KeyCode.F1 + (uint)(virtualKey - 0x70),
        >= 0x60 and <= 0x69 => KeyCode.Keypad0 + (uint)(virtualKey - 0x60),
        0x6E => KeyCode.Keypad0 + 10,
        0x6F => KeyCode.Keypad0 + 11,
        0x6A => KeyCode.Keypad0 + 12,
        0x6D => KeyCode.Keypad0 + 13,
        0x6B => KeyCode.Keypad0 + 14,
        0x6C => KeyCode.KeypadSeparator,
        0xB3 => KeyCode.MediaPlay + 2,
        0xB2 => KeyCode.MediaPlay + 4,
        0xB0 => KeyCode.MediaPlay + 7,
        0xB1 => KeyCode.MediaPlay + 8,
        0xAE => KeyCode.MediaPlay + 10,
        0xAF => KeyCode.MediaPlay + 11,
        0xAD => KeyCode.MediaPlay + 12,
        // WinUI names Shift, Control and Alt without a side: Shift's scan code
        // and the others' extended bit give it.
        0x10 => scanCode == 0x36 ? KeyCode.RightShift : KeyCode.LeftShift,
        0x11 => (extended ? KeyCode.RightShift : KeyCode.LeftShift) + 1,
        0x12 => (extended ? KeyCode.RightShift : KeyCode.LeftShift) + 2,
        >= 0xA0 and <= 0xA5 => ((virtualKey & 1) == 0 ? KeyCode.LeftShift : KeyCode.RightShift) + (uint)((virtualKey - 0xA0) / 2),
        0x5B => KeyCode.LeftShift + 3,
        0x5C => KeyCode.RightShift + 3,
        // An input method holds the key (VK_PROCESSKEY) or SendInput typed a
        // character (VK_PACKET): whatever text follows is not this key's.
        0xE5 or 0xE7 => null,
        _ => extended ? null : MainBlockKey(scanCode),
    };

    /// <summary>Whether a key with these modifiers held is a paste shortcut: Ctrl+Shift+V or Shift+Insert.</summary>
    public static bool IsPaste(uint? key, KeyModifiers held) => (key, held & Shortcut) switch
    {
        ((uint)'v', KeyModifiers.Control | KeyModifiers.Shift) or (KeyCode.Insert, KeyModifiers.Shift) => true,
        _ => false,
    };

    /// <summary>
    /// The modifiers held, given each virtual key's <c>CoreVirtualKeyStates</c>
    /// (1 down, 2 locked) as <c>InputKeyboardSource.GetKeyStateForCurrentThread</c> reports it.
    /// AltGr shows as Ctrl+Alt here, which <see cref="WindowsKeyPairing"/> relies on.
    /// </summary>
    public static KeyModifiers Held(Func<int, int> state)
    {
        KeyModifiers held = KeyModifiers.None;
        foreach ((int virtualKey, int bit, KeyModifiers modifier) in HeldKeys)
        {
            if ((state(virtualKey) & bit) != 0)
            {
                held |= modifier;
            }
        }
        return held;
    }

    /// <summary>The modifier a modifier key holds, which the core counts as held during its own event.</summary>
    internal static KeyModifiers ModifierOf(uint key)
    {
        // Wraps below Left Shift, as in MainBlockKey.
        uint side = key - KeyCode.LeftShift;
        return side < 2 * SideModifiers.Length ? SideModifiers[side % SideModifiers.Length] : KeyModifiers.None;
    }

    private static uint? MainBlockKey(uint scanCode)
    {
        foreach ((uint first, string keys) in MainBlock)
        {
            // Wraps for codes below the span, so one comparison covers both ends.
            uint offset = scanCode - first;
            if (offset < keys.Length)
            {
                return keys[(int)offset];
            }
        }
        return null;
    }
}

/// <summary>
/// Pairs WinUI's <c>KeyDown</c> with the <c>CharacterReceived</c> that follows
/// it, because a key event carries the text the key typed and Windows reports
/// the two apart. A text key waits for its character. Control, Alt, Super and
/// keys that type nothing go out at once; the characters Windows makes of them
/// (Ctrl+A's 0x01, Enter's CR) are the core's to make, so they are dropped.
/// Ctrl+Alt is AltGr when a character follows, which then keeps both as consumed.
/// </summary>
public sealed class WindowsKeyPairing(Action<KeyInput> sendKey, Action<string> sendText)
{
    private const KeyModifiers AltGr = KeyModifiers.Control | KeyModifiers.Alt;
    private const KeyModifiers Chord = AltGr | KeyModifiers.Super;

    private KeyInput? pending;
    private bool dropping;
    private char high;

    /// <summary>
    /// A key went down. True when it was sent, and the host marks the event
    /// handled (which also keeps Tab and the arrows from moving focus); false
    /// while it waits for its character or names no key.
    /// </summary>
    public bool KeyDown(int virtualKey, uint scanCode, bool extended, bool repeat, KeyModifiers held)
    {
        Flush();
        dropping = false;
        if (WindowsKeys.Map(virtualKey, scanCode, extended) is not uint key)
        {
            // Its character, if any, arrives on its own and goes out as text.
            return false;
        }
        held |= WindowsKeys.ModifierOf(key);
        var input = new KeyInput(key, repeat ? KeyAction.Repeat : KeyAction.Press, held);
        KeyModifiers chord = held & Chord;
        if (KeyCode.TypesText(key) && (chord == 0 || chord == AltGr))
        {
            pending = input;
            return false;
        }
        sendKey(input);
        dropping = true;
        return true;
    }

    /// <summary>A key went up; true when a release was sent.</summary>
    public bool KeyUp(int virtualKey, uint scanCode, bool extended, KeyModifiers held)
    {
        Flush();
        if (WindowsKeys.Map(virtualKey, scanCode, extended) is not uint key)
        {
            return false;
        }
        sendKey(new KeyInput(key, KeyAction.Release, held | WindowsKeys.ModifierOf(key)));
        return true;
    }

    /// <summary>One UTF-16 unit of <c>CharacterReceived</c>; a surrogate pair arrives as two.</summary>
    public void Character(char unit)
    {
        string text;
        if (char.IsHighSurrogate(unit))
        {
            high = unit;
            return;
        }
        if (char.IsLowSurrogate(unit))
        {
            if (high == default)
            {
                return;
            }
            text = char.ConvertFromUtf32(char.ConvertToUtf32(high, unit));
        }
        else
        {
            text = unit.ToString();
        }
        high = default;
        if (dropping || char.IsControl(unit))
        {
            return;
        }
        if (pending is KeyInput key)
        {
            pending = null;
            sendKey(key with { Text = text, Consumed = key.Modifiers & AltGr });
        }
        else
        {
            sendText(text);
        }
    }

    // A text key that saw no character: a dead key waiting for the next one
    // (its character comes with that key), unless Ctrl+Alt made it a chord.
    private void Flush()
    {
        if (pending is KeyInput key && (key.Modifiers & AltGr) == AltGr)
        {
            sendKey(key);
        }
        pending = null;
        high = default;
    }
}

/// <summary>
/// Wheel deltas to whole steps. Windows reports 120 per notch and a precision
/// touchpad a fraction of that per event, so what is left over carries to the next.
/// </summary>
public sealed class WheelSteps
{
    public const int Notch = 120;

    private int remainder;

    /// <summary>Steps for <paramref name="delta"/>: positive away from the user (up, or right when horizontal).</summary>
    public int Add(int delta)
    {
        long total = (long)remainder + delta;
        remainder = (int)(total % Notch);
        return (int)(total / Notch);
    }
}

/// <summary>
/// WinUI pointer updates to mouse events. Windows raises <c>PointerPressed</c>
/// for the first button only and reports the others through <c>PointerMoved</c>,
/// so every update goes through <see cref="Update"/>, whichever event carried it.
/// </summary>
public sealed class WindowsPointer
{
    // PointerUpdateKind from LeftButtonPressed (1): a press, then a release, per button.
    private static readonly MouseButton[] Buttons = [MouseButton.Left, MouseButton.Right, MouseButton.Middle, MouseButton.Back, MouseButton.Forward];

    private (uint Col, uint Row)? last;

    /// <summary>
    /// The event for <c>PointerUpdateKind</c> <paramref name="updateKind"/> at
    /// <paramref name="at"/> (from <see cref="PointerCells.At"/>). Any other kind is a
    /// move: motion dragging the held button (left, middle, right), or null while
    /// the pointer stays in the cell of the last event, as programs only see cells.
    /// </summary>
    public MouseInput? Update(int updateKind, bool left, bool middle, bool right, KeyModifiers modifiers, (uint Col, uint Row, uint XPx, uint YPx) at)
    {
        MouseAction action;
        MouseButton button;
        if (updateKind is >= 1 and <= 10)
        {
            action = (updateKind & 1) == 1 ? MouseAction.Press : MouseAction.Release;
            button = Buttons[(updateKind - 1) / 2];
        }
        else if (last == (at.Col, at.Row))
        {
            return null;
        }
        else
        {
            action = MouseAction.Motion;
            button = left ? MouseButton.Left : middle ? MouseButton.Middle : right ? MouseButton.Right : MouseButton.None;
        }
        last = (at.Col, at.Row);
        return new MouseInput(action, button, modifiers, at.Col, at.Row, at.XPx, at.YPx);
    }

    /// <summary>One wheel step in the direction of <paramref name="steps"/> (from <see cref="WheelSteps"/>): a press of a wheel button.</summary>
    public MouseInput Wheel(int steps, bool horizontal, KeyModifiers modifiers, (uint Col, uint Row, uint XPx, uint YPx) at)
    {
        MouseButton button = (steps > 0, horizontal) switch
        {
            (true, false) => MouseButton.WheelUp,
            (false, false) => MouseButton.WheelDown,
            (true, true) => MouseButton.WheelRight,
            (false, true) => MouseButton.WheelLeft,
        };
        last = (at.Col, at.Row);
        return new MouseInput(MouseAction.Press, button, modifiers, at.Col, at.Row, at.XPx, at.YPx);
    }
}

/// <summary>Pointer positions to cells.</summary>
public static class PointerCells
{
    /// <summary>
    /// The viewport cell under a point in pixels from the text area's top left,
    /// clamped to the grid, and the point itself clamped to the area's origin.
    /// </summary>
    public static (uint Col, uint Row, uint XPx, uint YPx) At(double x, double y, int cellWidth, int cellHeight, int cols, int rows)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(cellWidth);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(cellHeight);
        uint px = Pixel(x), py = Pixel(y);
        return ((uint)Math.Min(px / cellWidth, (uint)Math.Max(cols - 1, 0)), (uint)Math.Min(py / cellHeight, (uint)Math.Max(rows - 1, 0)), px, py);
    }

    // A pointer captured outside the window reports negative or huge positions.
    private static uint Pixel(double value) => double.IsFinite(value) ? (uint)Math.Clamp(value, 0, uint.MaxValue) : 0;
}
