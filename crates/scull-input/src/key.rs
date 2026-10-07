//! The platform-neutral key event every key encoder reads, and the tables
//! that name keys on the wire. Separate from the encoders so the C ABI can
//! mirror one plain struct and no UI-toolkit key type reaches the core.
//!
//! The fields follow Ghostty's embedding API (`docs/research/ffi-native-ui.md`):
//! the platform reports what was pressed and what text it produced; the core
//! decides the bytes.

use bitflags::bitflags;

use crate::bytes::{DECIMAL_RADIX, ESC};

bitflags! {
    /// Modifier state. The bits are the kitty protocol's, which extend
    /// xterm's (shift 1, alt 2, ctrl 4, meta/super 8), so every encoding
    /// sends `bits + 1`.
    /// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#modifiers>
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        /// Shift.
        const SHIFT = 1;
        /// Alt, Option on macOS.
        const ALT = 1 << 1;
        /// Control.
        const CTRL = 1 << 2;
        /// Super: the Windows key or Command on macOS.
        const SUPER = 1 << 3;
        /// Hyper (X11/Wayland only).
        const HYPER = 1 << 4;
        /// Meta (X11/Wayland only).
        const META = 1 << 5;
        /// Caps Lock is on.
        const CAPS_LOCK = 1 << 6;
        /// Num Lock is on.
        const NUM_LOCK = 1 << 7;
    }
}

impl Modifiers {
    /// The lock states, which legacy encodings cannot carry.
    pub const LOCKS: Self = Self::CAPS_LOCK.union(Self::NUM_LOCK);
    /// Modifiers xterm has no legacy spelling for in front of a text key.
    pub(crate) const BEYOND_XTERM: Self = Self::SUPER.union(Self::HYPER).union(Self::META);

    /// The value on the wire: kitty and xterm both send `1 + bits` so that
    /// an absent parameter (1) means "no modifiers".
    pub(crate) fn wire(self) -> u32 {
        u32::from(self.bits()) + 1
    }
}

/// What happened to the key. Discriminants are the kitty event types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum KeyAction {
    /// The key went down.
    #[default]
    Press = 1,
    /// The key is held and auto-repeats.
    Repeat = 2,
    /// The key went up.
    Release = 3,
}

/// Keypad keys. Discriminants are the kitty functional key codes, so the
/// encoder needs no second table.
/// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions>
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum KeypadKey {
    Digit0 = 57_399,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Decimal,
    Divide,
    Multiply,
    Subtract,
    Add,
    Enter,
    Equal,
    Separator,
    // Navigation: what the keypad sends when Num Lock is off.
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    Begin,
}

impl KeypadKey {
    /// The character the key types, for keys that type one.
    pub(crate) fn text_char(self) -> Option<char> {
        let digit = (self as u32).checked_sub(Self::Digit0 as u32);
        match self {
            Self::Decimal => Some('.'),
            Self::Divide => Some('/'),
            Self::Multiply => Some('*'),
            Self::Subtract => Some('-'),
            Self::Add => Some('+'),
            Self::Equal => Some('='),
            Self::Separator => Some(','),
            _ => digit.and_then(|d| char::from_digit(d, DECIMAL_RADIX)),
        }
    }
}

/// Media and volume keys; discriminants are the kitty codes.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MediaKey {
    Play = 57_428,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

/// Modifier keys pressed on their own; discriminants are the kitty codes.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ModifierKey {
    LeftShift = 57_441,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

/// Lock and system keys; discriminants are the kitty codes.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SystemKey {
    CapsLock = 57_358,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
}

/// Which key was pressed, independent of the keyboard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key of the main block named by what it types on the US (PC-101)
    /// layout without modifiers: `Char('a')`, `Char('1')`, `Char(' ')`.
    /// This is kitty's "base layout key", which keeps `Ctrl+C` working on
    /// layouts where the same key types `с`.
    Char(char),
    /// Escape.
    Escape,
    /// Enter / Return of the main block.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Insert.
    Insert,
    /// Forward delete.
    Delete,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Function key F1..=F35; other numbers encode to nothing.
    F(u8),
    /// A keypad key.
    Keypad(KeypadKey),
    /// A media or volume key.
    Media(MediaKey),
    /// A modifier key on its own.
    Modifier(ModifierKey),
    /// A lock or system key.
    System(SystemKey),
}

/// One key event from the platform. Borrowed text keeps encoding
/// allocation-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent<'a> {
    /// The key.
    pub key: Key,
    /// Press, repeat or release.
    pub action: KeyAction,
    /// Modifiers held, including the effect of this event when the key is
    /// itself a modifier (kitty requires it).
    pub mods: Modifiers,
    /// Modifiers the layout used up to produce `text` (Option on macOS
    /// typing `å`). They are not reported, except Shift, which every
    /// protocol reports alongside the shifted text.
    pub consumed: Modifiers,
    /// The text the key types with Shift and consumed modifiers applied
    /// but without Ctrl, whose effect the encoders compute. Control
    /// characters in it are ignored.
    pub text: &'a str,
    /// The code point the key types on the current layout without
    /// modifiers (`с` for `Char('c')` on a Russian layout). `None` means
    /// the US layout's character from [`Key::Char`].
    pub unshifted: Option<char>,
    /// An input method is composing; the event must not reach the program.
    pub composing: bool,
}

impl<'a> KeyEvent<'a> {
    /// A press of `key` with no modifiers and no text.
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            action: KeyAction::Press,
            mods: Modifiers::empty(),
            consumed: Modifiers::empty(),
            text: "",
            unshifted: None,
            composing: false,
        }
    }

    /// Modifiers to report: everything held except what the layout used up,
    /// Shift excepted.
    pub(crate) fn reported_mods(&self) -> Modifiers {
        self.mods - self.consumed.difference(Modifiers::SHIFT)
    }

    /// The text with control characters dropped, so a platform bug cannot
    /// smuggle an escape sequence through a key event.
    pub(crate) fn clean_text(&self) -> impl Iterator<Item = char> + 'a {
        self.text.chars().filter(|c| !c.is_control())
    }

    pub(crate) fn has_text(&self) -> bool {
        self.clean_text().next().is_some()
    }

    /// The single character typed, if the text is exactly one character.
    pub(crate) fn single_char(&self) -> Option<char> {
        let mut chars = self.clean_text();
        chars.next().filter(|_| chars.next().is_none())
    }
}

/// Final byte of a `CSI number u` report.
pub(crate) const FINAL_U: u8 = b'u';
/// Final byte of a `CSI number ~` report.
pub(crate) const FINAL_TILDE: u8 = b'~';

/// Number given to the arrows, Home, End and F1–F4, whose final letter
/// names the key; it is omitted when no modifiers follow.
pub(crate) const LETTER_KEY_NUMBER: u32 = 1;
/// F1 is `CSI 1 P`; F13 is the first function key with a private-use code.
const F13: u8 = 13;
const F13_CODE: u32 = 57_376;
const LAST_F_KEY: u8 = 35;
/// `CSI n ~` numbers of F5–F12; the gaps are DEC's.
const F5_TO_F12: [u32; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
const F5: u8 = 5;

/// How the kitty functional key table spells `key`: `CSI number final`.
/// `None` for keys of the main block (they report their code point) and
/// for function keys outside F1..=F35.
/// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions>
pub(crate) fn functional(key: Key) -> Option<(u32, u8)> {
    let letter = |l| Some((LETTER_KEY_NUMBER, l));
    let tilde = |n| Some((n, FINAL_TILDE));
    let code = |n| Some((n, FINAL_U));
    match key {
        Key::Char(_) => None,
        Key::Escape => code(u32::from(ESC)),
        Key::Enter => code(u32::from(CR)),
        Key::Tab => code(u32::from(TAB)),
        Key::Backspace => code(u32::from(DEL)),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Up => letter(b'A'),
        Key::Down => letter(b'B'),
        Key::Right => letter(b'C'),
        Key::Left => letter(b'D'),
        Key::End => letter(b'F'),
        Key::Home => letter(b'H'),
        Key::F(1) => letter(b'P'),
        Key::F(2) => letter(b'Q'),
        Key::F(3) => tilde(13),
        Key::F(4) => letter(b'S'),
        Key::F(n @ F5..F13) => F5_TO_F12.get(usize::from(n - F5)).and_then(|&n| tilde(n)),
        Key::F(n @ F13..=LAST_F_KEY) => code(F13_CODE + u32::from(n - F13)),
        Key::F(_) => None,
        Key::Keypad(KeypadKey::Begin) => letter(b'E'),
        Key::Keypad(k) => code(k as u32),
        Key::Media(k) => code(k as u32),
        Key::Modifier(k) => code(k as u32),
        Key::System(k) => code(k as u32),
    }
}

/// Carriage return: Enter.
pub(crate) const CR: u8 = b'\r';
/// Horizontal tab: Tab.
pub(crate) const TAB: u8 = b'\t';
/// DEL: Backspace.
pub(crate) const DEL: u8 = 0x7f;
/// Backspace with Ctrl.
pub(crate) const BS: u8 = 0x08;
/// Ctrl keeps the low five bits of `@ A–Z [ \ ] ^ _`.
const CTRL_MASK: u8 = 0x1f;

/// The byte Ctrl+`c` sends, from kitty's superset of the VT100 table;
/// `None` for keys Ctrl leaves alone.
/// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#legacy-ctrl-mapping-of-ascii-keys>
pub(crate) fn ctrl_byte(c: char) -> Option<u8> {
    let b = u8::try_from(c).ok()?;
    Some(match c {
        'a'..='z' | '@' | '[' | '\\' | ']' | '^' | '_' => b & CTRL_MASK,
        ' ' | '2' => b'@' & CTRL_MASK,
        '3'..='7' => b - b'3' + ESC,
        '/' => b'_' & CTRL_MASK,
        '~' => b'^' & CTRL_MASK,
        '8' | '?' => DEL,
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A press of `key` with `mods` held, typing `text`.
    pub(crate) fn event(key: Key, mods: Modifiers, text: &str) -> KeyEvent<'_> {
        KeyEvent {
            mods,
            text,
            ..KeyEvent::new(key)
        }
    }

    /// Modifier combinations by name, as the specification's tables label
    /// their columns.
    pub(crate) const NONE: Modifiers = Modifiers::empty();
    pub(crate) const SHIFT: Modifiers = Modifiers::SHIFT;
    pub(crate) const ALT: Modifiers = Modifiers::ALT;
    pub(crate) const CTRL: Modifiers = Modifiers::CTRL;
    pub(crate) const CTRL_SHIFT: Modifiers = CTRL.union(SHIFT);
    pub(crate) const ALT_SHIFT: Modifiers = ALT.union(SHIFT);
    pub(crate) const CTRL_ALT: Modifiers = CTRL.union(ALT);

    #[test]
    fn ctrl_table_matches_the_kitty_specification() {
        let table = [
            (' ', 0),
            ('/', 31),
            ('2', 0),
            ('3', 27),
            ('4', 28),
            ('5', 29),
            ('6', 30),
            ('7', 31),
            ('8', 127),
            ('?', 127),
            ('@', 0),
            ('[', 27),
            ('\\', 28),
            (']', 29),
            ('^', 30),
            ('_', 31),
            ('a', 1),
            ('i', 9),
            ('m', 13),
            ('z', 26),
            ('~', 30),
        ];
        for (key, byte) in table {
            assert_eq!(ctrl_byte(key), Some(byte), "ctrl+{key}");
        }
        for untouched in ['0', '1', '9', ';', '\'', ',', '.', 'A', 'ж'] {
            assert_eq!(ctrl_byte(untouched), None, "ctrl+{untouched}");
        }
    }

    #[test]
    fn functional_table_matches_the_kitty_specification() {
        let table = [
            (Key::Escape, 27, b'u'),
            (Key::Enter, 13, b'u'),
            (Key::Tab, 9, b'u'),
            (Key::Backspace, 127, b'u'),
            (Key::Insert, 2, b'~'),
            (Key::Delete, 3, b'~'),
            (Key::Left, 1, b'D'),
            (Key::PageDown, 6, b'~'),
            (Key::Home, 1, b'H'),
            (Key::End, 1, b'F'),
            (Key::System(SystemKey::CapsLock), 57_358, b'u'),
            (Key::System(SystemKey::Menu), 57_363, b'u'),
            (Key::F(1), 1, b'P'),
            (Key::F(3), 13, b'~'),
            (Key::F(5), 15, b'~'),
            (Key::F(6), 17, b'~'),
            (Key::F(11), 23, b'~'),
            (Key::F(12), 24, b'~'),
            (Key::F(13), 57_376, b'u'),
            (Key::F(35), 57_398, b'u'),
            (Key::Keypad(KeypadKey::Digit0), 57_399, b'u'),
            (Key::Keypad(KeypadKey::Enter), 57_414, b'u'),
            (Key::Keypad(KeypadKey::Delete), 57_426, b'u'),
            (Key::Keypad(KeypadKey::Begin), 1, b'E'),
            (Key::Media(MediaKey::Play), 57_428, b'u'),
            (Key::Media(MediaKey::MuteVolume), 57_440, b'u'),
            (Key::Modifier(ModifierKey::LeftShift), 57_441, b'u'),
            (Key::Modifier(ModifierKey::IsoLevel5Shift), 57_454, b'u'),
        ];
        for (key, number, fin) in table {
            assert_eq!(functional(key), Some((number, fin)), "{key:?}");
        }
        assert_eq!(functional(Key::F(0)), None);
        assert_eq!(functional(Key::F(36)), None);
        assert_eq!(functional(Key::Char('a')), None);
    }

    #[test]
    fn keypad_text_keys_type_their_characters() {
        assert_eq!(KeypadKey::Digit0.text_char(), Some('0'));
        assert_eq!(KeypadKey::Digit9.text_char(), Some('9'));
        assert_eq!(KeypadKey::Separator.text_char(), Some(','));
        assert_eq!(KeypadKey::Left.text_char(), None);
        assert_eq!(KeypadKey::Enter.text_char(), None);
    }

    #[test]
    fn consumed_modifiers_are_not_reported_except_shift() {
        let mut event = KeyEvent::new(Key::Char('a'));
        event.mods = Modifiers::SHIFT | Modifiers::ALT | Modifiers::CTRL;
        event.consumed = Modifiers::SHIFT | Modifiers::ALT;
        assert_eq!(event.reported_mods(), Modifiers::SHIFT | Modifiers::CTRL);
    }

    #[test]
    fn control_characters_in_text_are_ignored() {
        let mut event = KeyEvent::new(Key::Char('a'));
        event.text = "\x1b[201~";
        assert_eq!(event.clean_text().collect::<String>(), "[201~");
        event.text = "\x01";
        assert!(!event.has_text());
    }
}
