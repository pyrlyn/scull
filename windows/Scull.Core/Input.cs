using Scull.Native;

namespace Scull.Core;

/// <summary>The <c>TT_MOD_*</c> bits of a key or mouse event.</summary>
[Flags]
public enum KeyModifiers : byte
{
    None = 0,
    Shift = NativeMethods.TT_MOD_SHIFT,
    Alt = NativeMethods.TT_MOD_ALT,
    Control = NativeMethods.TT_MOD_CTRL,
    Super = NativeMethods.TT_MOD_SUPER,
    Hyper = NativeMethods.TT_MOD_HYPER,
    Meta = NativeMethods.TT_MOD_META,
    CapsLock = NativeMethods.TT_MOD_CAPS_LOCK,
    NumLock = NativeMethods.TT_MOD_NUM_LOCK,
}

public enum KeyAction : byte
{
    Press = NativeMethods.TT_KEY_PRESS,
    Repeat = NativeMethods.TT_KEY_REPEAT,
    Release = NativeMethods.TT_KEY_RELEASE,
}

/// <summary>
/// <c>tt_key_event.key</c> codes. A main-block key is the code point it types
/// on the US layout with no modifiers; the rest follow one another in the
/// order scull.h lists them, so each group is its first code plus an offset.
/// </summary>
public static class KeyCode
{
    public const uint Escape = NativeMethods.TT_KEY_ESCAPE;
    public const uint Enter = Escape + 1;
    public const uint Tab = Escape + 2;
    public const uint Backspace = Escape + 3;
    public const uint Insert = Escape + 4;
    public const uint Delete = Escape + 5;
    public const uint Left = Escape + 6;
    public const uint Right = Escape + 7;
    public const uint Up = Escape + 8;
    public const uint Down = Escape + 9;
    public const uint PageUp = Escape + 10;
    public const uint PageDown = Escape + 11;
    public const uint Home = Escape + 12;
    public const uint End = NativeMethods.TT_KEY_END;
    public const uint CapsLock = NativeMethods.TT_KEY_CAPS_LOCK;
    public const uint ScrollLock = CapsLock + 1;
    public const uint NumLock = CapsLock + 2;
    public const uint PrintScreen = CapsLock + 3;
    public const uint Pause = CapsLock + 4;
    public const uint Menu = CapsLock + 5;
    public const uint F1 = NativeMethods.TT_KEY_F1;

    /// <summary>Keypad 0; then 1 to 9, Decimal, Divide, Multiply, Subtract, Add, Enter, Equal, Separator, Left, Right, Up, Down, Page Up, Page Down, Home, End, Insert, Delete, Begin.</summary>
    public const uint Keypad0 = NativeMethods.TT_KEY_KP_0;
    public const uint KeypadEnter = Keypad0 + 15;
    public const uint KeypadSeparator = Keypad0 + 17;
    public const uint KeypadLeft = Keypad0 + 18;

    /// <summary>Play; then Pause, Play/Pause, Reverse, Stop, Fast Forward, Rewind, Next Track, Previous Track, Record, Volume Down, Volume Up, Mute.</summary>
    public const uint MediaPlay = NativeMethods.TT_KEY_MEDIA_PLAY;

    /// <summary>Left Shift; then Left Control, Alt, Super, Hyper, Meta, and the same six on the right.</summary>
    public const uint LeftShift = NativeMethods.TT_KEY_LEFT_SHIFT;
    public const uint RightShift = LeftShift + 6;

    /// <summary>Whether the key types text: a main-block key, or a keypad digit or operator.</summary>
    public static bool TypesText(uint key) =>
        key < Escape || (key >= Keypad0 && key <= KeypadSeparator && key != KeypadEnter);
}

/// <summary>
/// One key event. <paramref name="Text"/> is what the key typed, with Shift and
/// <paramref name="Consumed"/> applied but not Control; <paramref name="Consumed"/>
/// are the modifiers the layout used to type it (AltGr's Ctrl+Alt).
/// </summary>
public readonly record struct KeyInput(uint Key, KeyAction Action, KeyModifiers Modifiers, KeyModifiers Consumed = KeyModifiers.None, string Text = "");

public enum MouseAction : byte
{
    Press = NativeMethods.TT_MOUSE_PRESS,
    Release = NativeMethods.TT_MOUSE_RELEASE,
    Motion = NativeMethods.TT_MOUSE_MOTION,
}

/// <summary>X11's button numbers; a wheel step is a press of a wheel button.</summary>
public enum MouseButton : byte
{
    None = NativeMethods.TT_MOUSE_NONE,
    Left = NativeMethods.TT_MOUSE_LEFT,
    Middle = NativeMethods.TT_MOUSE_MIDDLE,
    Right = NativeMethods.TT_MOUSE_RIGHT,
    WheelUp = NativeMethods.TT_MOUSE_WHEEL_UP,
    WheelDown = NativeMethods.TT_MOUSE_WHEEL_DOWN,
    WheelLeft = NativeMethods.TT_MOUSE_WHEEL_LEFT,
    WheelRight = NativeMethods.TT_MOUSE_WHEEL_RIGHT,
    Back = 8,
    Forward = 9,
}

/// <summary>One mouse event at a viewport cell and at a pixel of the text area; the core picks which the program gets.</summary>
public readonly record struct MouseInput(MouseAction Action, MouseButton Button, KeyModifiers Modifiers, uint Col, uint Row, uint XPx, uint YPx);
