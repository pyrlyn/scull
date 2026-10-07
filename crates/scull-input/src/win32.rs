//! win32-input-mode (`CSI ? 9001 h`): every key event, down and up, as the
//! fields of a Windows `KEY_EVENT_RECORD`, `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`.
//! ConPTY sets the mode so Windows console programs get the input records
//! they expect. Separate because it ignores every other key mode: ConPTY
//! rebuilds VT sequences from the record itself.
//! <https://github.com/microsoft/terminal/blob/main/doc/specs/%234999%20-%20Improved%20keyboard%20handling%20in%20Conpty.md>
//!
//! The virtual-key and scan codes are derived from [`Key`] on the US
//! layout, so the encoder stays platform-neutral.
//! Virtual keys: <https://learn.microsoft.com/windows/win32/inputdev/virtual-key-codes>;
//! scan codes are PC/AT set 1.

use crate::bytes::{CSI, ESC, SEMI, push_decimal};
use crate::key::{
    BS, CR, Key, KeyAction, KeyEvent, KeypadKey as Kp, MediaKey, ModifierKey, Modifiers, SystemKey,
    TAB, ctrl_byte,
};

/// `dwControlKeyState` bits (wincon.h). Left and right are not tracked in
/// [`Modifiers`], so held modifiers report as the left key.
const LEFT_ALT_PRESSED: u32 = 0x0002;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const SHIFT_PRESSED: u32 = 0x0010;
const NUMLOCK_ON: u32 = 0x0020;
const CAPSLOCK_ON: u32 = 0x0080;
const ENHANCED_KEY: u32 = 0x0100;

/// Final byte of a win32-input-mode record.
const FINAL: u8 = b'_';
/// Each record stands for one keystroke.
const REPEAT_COUNT: u32 = 1;

/// A key's Windows identity: virtual key, scan code and whether it is an
/// extended (E0-prefixed) key.
#[derive(Clone, Copy)]
struct Win32Key(u16, u16, bool);

const UNKNOWN: Win32Key = Win32Key(0, 0, false);
const fn std_key(vk: u16, scan: u16) -> Win32Key {
    Win32Key(vk, scan, false)
}
const fn ext_key(vk: u16, scan: u16) -> Win32Key {
    Win32Key(vk, scan, true)
}

/// Main-block rows of scan code set 1: first code and the US keys in order.
const SCAN_ROWS: [(u16, &str); 4] = [
    (0x02, "1234567890-="),
    (0x10, "qwertyuiop[]"),
    (0x1E, "asdfghjkl;'`"),
    (0x2C, "zxcvbnm,./"),
];
const SCAN_BACKSLASH: u16 = 0x2B;
const SCAN_SPACE: u16 = 0x39;
/// `VK_OEM_*` codes of the US punctuation keys.
const OEM_VKS: [(char, u16); 11] = [
    (';', 0xBA),
    ('=', 0xBB),
    (',', 0xBC),
    ('-', 0xBD),
    ('.', 0xBE),
    ('/', 0xBF),
    ('`', 0xC0),
    ('[', 0xDB),
    ('\\', 0xDC),
    (']', 0xDD),
    ('\'', 0xDE),
];
/// Keypad digit scan codes, indexed by digit.
const KEYPAD_DIGIT_SCANS: [u16; 10] = [0x52, 0x4F, 0x50, 0x51, 0x4B, 0x4C, 0x4D, 0x47, 0x48, 0x49];
const VK_NUMPAD0: u16 = 0x60;
/// Virtual keys run F1..=F24 without a gap; scan codes come in three
/// consecutive runs (F1–F10, F11–F12, F13–F23) and a lone F24.
const VK_F1: u16 = 0x70;
const F1: u8 = 1;
const F10: u8 = 10;
const F11: u8 = 11;
const F12: u8 = 12;
const F13: u8 = 13;
const F23: u8 = 23;
const F24: u8 = 24;
const SCAN_F1: u16 = 0x3B;
const SCAN_F11: u16 = 0x57;
const SCAN_F13: u16 = 0x64;
const SCAN_F24: u16 = 0x76;

fn char_key(c: char) -> Win32Key {
    let lower = c.to_ascii_lowercase();
    let scan = match lower {
        ' ' => SCAN_SPACE,
        '\\' => SCAN_BACKSLASH,
        _ => SCAN_ROWS
            .iter()
            .find_map(|(first, row)| {
                let at = row.chars().position(|k| k == lower)?;
                Some(first + u16::try_from(at).ok()?)
            })
            .unwrap_or_default(),
    };
    let vk = if lower.is_ascii_alphanumeric() || lower == ' ' {
        u16::from(lower.to_ascii_uppercase() as u8)
    } else {
        OEM_VKS
            .iter()
            .find(|(k, _)| *k == lower)
            .map_or(0, |&(_, vk)| vk)
    };
    std_key(vk, scan)
}

fn function_key(n: u8) -> Win32Key {
    let vk = |n: u8| VK_F1 + u16::from(n - F1);
    match n {
        F1..=F10 => std_key(vk(n), SCAN_F1 + u16::from(n - F1)),
        F11..=F12 => std_key(vk(n), SCAN_F11 + u16::from(n - F11)),
        F13..=F23 => std_key(vk(n), SCAN_F13 + u16::from(n - F13)),
        F24 => std_key(vk(n), SCAN_F24),
        _ => UNKNOWN,
    }
}

fn win32_key(key: Key) -> Win32Key {
    match key {
        Key::Char(c) => char_key(c),
        Key::Escape => std_key(0x1B, 0x01),
        Key::Enter => std_key(0x0D, 0x1C),
        Key::Tab => std_key(0x09, 0x0F),
        Key::Backspace => std_key(0x08, 0x0E),
        Key::Insert => ext_key(0x2D, 0x52),
        Key::Delete => ext_key(0x2E, 0x53),
        Key::Left => ext_key(0x25, 0x4B),
        Key::Right => ext_key(0x27, 0x4D),
        Key::Up => ext_key(0x26, 0x48),
        Key::Down => ext_key(0x28, 0x50),
        Key::PageUp => ext_key(0x21, 0x49),
        Key::PageDown => ext_key(0x22, 0x51),
        Key::Home => ext_key(0x24, 0x47),
        Key::End => ext_key(0x23, 0x4F),
        Key::F(n) => function_key(n),
        Key::Keypad(k) => keypad_key(k),
        Key::Media(k) => match k {
            MediaKey::Play | MediaKey::Pause | MediaKey::PlayPause => ext_key(0xB3, 0x22),
            MediaKey::Stop => ext_key(0xB2, 0x24),
            MediaKey::TrackNext => ext_key(0xB0, 0x19),
            MediaKey::TrackPrevious => ext_key(0xB1, 0x10),
            MediaKey::LowerVolume => ext_key(0xAE, 0x2E),
            MediaKey::RaiseVolume => ext_key(0xAF, 0x30),
            MediaKey::MuteVolume => ext_key(0xAD, 0x20),
            MediaKey::Reverse | MediaKey::FastForward | MediaKey::Rewind | MediaKey::Record => {
                UNKNOWN
            }
        },
        // Windows reports the generic VK_SHIFT/VK_CONTROL/VK_MENU in records.
        Key::Modifier(k) => match k {
            ModifierKey::LeftShift => std_key(0x10, 0x2A),
            ModifierKey::RightShift => std_key(0x10, 0x36),
            ModifierKey::LeftControl => std_key(0x11, 0x1D),
            ModifierKey::RightControl => ext_key(0x11, 0x1D),
            ModifierKey::LeftAlt => std_key(0x12, 0x38),
            ModifierKey::RightAlt => ext_key(0x12, 0x38),
            ModifierKey::LeftSuper => ext_key(0x5B, 0x5B),
            ModifierKey::RightSuper => ext_key(0x5C, 0x5C),
            _ => UNKNOWN,
        },
        Key::System(k) => match k {
            SystemKey::CapsLock => std_key(0x14, 0x3A),
            SystemKey::ScrollLock => std_key(0x91, 0x46),
            SystemKey::NumLock => ext_key(0x90, 0x45),
            SystemKey::PrintScreen => ext_key(0x2C, 0x37),
            SystemKey::Pause => std_key(0x13, 0x45),
            SystemKey::Menu => ext_key(0x5D, 0x5D),
        },
    }
}

fn keypad_key(k: Kp) -> Win32Key {
    match k {
        Kp::Decimal => std_key(0x6E, 0x53),
        Kp::Divide => ext_key(0x6F, 0x35),
        Kp::Multiply => std_key(0x6A, 0x37),
        Kp::Subtract => std_key(0x6D, 0x4A),
        Kp::Add => std_key(0x6B, 0x4E),
        Kp::Enter => ext_key(0x0D, 0x1C),
        Kp::Equal => std_key(0x92, 0x59),
        Kp::Separator => std_key(0x6C, 0x7E),
        // With Num Lock off the keypad sends the navigation virtual keys
        // on its own (non-extended) scan codes.
        Kp::Left => std_key(0x25, 0x4B),
        Kp::Right => std_key(0x27, 0x4D),
        Kp::Up => std_key(0x26, 0x48),
        Kp::Down => std_key(0x28, 0x50),
        Kp::PageUp => std_key(0x21, 0x49),
        Kp::PageDown => std_key(0x22, 0x51),
        Kp::Home => std_key(0x24, 0x47),
        Kp::End => std_key(0x23, 0x4F),
        Kp::Insert => std_key(0x2D, 0x52),
        Kp::Delete => std_key(0x2E, 0x53),
        Kp::Begin => std_key(0x0C, 0x4C),
        digit => {
            let d = (digit as u32).saturating_sub(Kp::Digit0 as u32);
            let d16 = u16::try_from(d).unwrap_or_default();
            let scan = KEYPAD_DIGIT_SCANS
                .get(usize::from(d16))
                .copied()
                .unwrap_or_default();
            std_key(VK_NUMPAD0 + d16, scan)
        }
    }
}

fn control_key_state(mods: Modifiers, enhanced: bool) -> u32 {
    [
        (Modifiers::ALT, LEFT_ALT_PRESSED),
        (Modifiers::CTRL, LEFT_CTRL_PRESSED),
        (Modifiers::SHIFT, SHIFT_PRESSED),
        (Modifiers::NUM_LOCK, NUMLOCK_ON),
        (Modifiers::CAPS_LOCK, CAPSLOCK_ON),
    ]
    .iter()
    .filter(|(m, _)| mods.contains(*m))
    .fold(if enhanced { ENHANCED_KEY } else { 0 }, |acc, (_, bit)| {
        acc | bit
    })
}

pub(crate) fn encode(event: &KeyEvent<'_>, out: &mut Vec<u8>) {
    let Win32Key(vk, scan, enhanced) = win32_key(event.key);
    let down = event.action != KeyAction::Release;
    // The record carries every modifier held; Windows reports AltGr as
    // Ctrl+Alt even when the layout used it up.
    let state = control_key_state(event.mods, enhanced);
    let reported = event.reported_mods();
    let record = |unit: u16, out: &mut Vec<u8>| {
        out.extend_from_slice(&CSI);
        for (i, field) in [
            u32::from(vk),
            u32::from(scan),
            u32::from(unit),
            u32::from(down),
            state,
            REPEAT_COUNT,
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                out.push(SEMI);
            }
            push_decimal(out, field);
        }
        out.push(FINAL);
    };
    // Enter, Esc, Tab and Backspace carry their C0 byte, which the text
    // filter would drop.
    let c0 = match event.key {
        Key::Enter | Key::Keypad(Kp::Enter) => Some(CR),
        Key::Escape => Some(ESC),
        Key::Tab => Some(TAB),
        Key::Backspace => Some(BS),
        _ => None,
    };
    if let Some(byte) = c0 {
        return record(u16::from(byte), out);
    }
    // Windows puts the control character in UnicodeChar for Ctrl+key and
    // nothing for Ctrl+Alt+key unless the layout typed text with it.
    let ctrl = reported.contains(Modifiers::CTRL);
    if ctrl && !reported.contains(Modifiers::ALT) {
        let c = match event.key {
            Key::Char(base) => event.unshifted.filter(char::is_ascii).unwrap_or(base),
            _ => '\0',
        };
        return record(ctrl_byte(c).map_or(0, u16::from), out);
    }
    if ctrl || !event.has_text() {
        return record(0, out);
    }
    // Text outside the BMP goes as one record per UTF-16 unit, as Windows
    // delivers surrogate pairs.
    let mut units = [0_u16; char::MAX_LEN_UTF16];
    for c in event.clean_text() {
        for &unit in c.encode_utf16(&mut units).iter() {
            record(unit, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeypadKey;
    use crate::key::tests::{ALT, CTRL, CTRL_ALT, NONE, SHIFT, event};

    fn enc(event: &KeyEvent<'_>) -> String {
        let mut out = Vec::new();
        encode(event, &mut out);
        String::from_utf8(out).unwrap()
    }

    fn released(mut e: KeyEvent<'_>) -> KeyEvent<'_> {
        e.action = KeyAction::Release;
        e
    }

    #[test]
    fn records_match_the_conpty_specification_examples() {
        // The spec's examples, with the fields it lets an encoder omit
        // written out (Uc 0, Kd 0, Cs 0, Rc 1).
        let table = [
            (event(Key::Char('a'), NONE, "a"), "65;30;97;1;0;1"),
            (released(event(Key::Char('a'), NONE, "a")), "65;30;97;0;0;1"),
            (event(Key::Char('a'), SHIFT, "A"), "65;30;65;1;16;1"),
            (
                event(Key::Modifier(ModifierKey::LeftShift), SHIFT, ""),
                "16;42;0;1;16;1",
            ),
            (
                event(Key::Modifier(ModifierKey::LeftControl), CTRL, ""),
                "17;29;0;1;8;1",
            ),
            (event(Key::F(1), CTRL, ""), "112;59;0;1;8;1"),
            (released(event(Key::F(1), CTRL, "")), "112;59;0;0;8;1"),
            (
                event(Key::Modifier(ModifierKey::LeftAlt), CTRL_ALT, ""),
                "18;56;0;1;10;1",
            ),
            (event(Key::Char('a'), CTRL_ALT, "a"), "65;30;0;1;10;1"),
        ];
        for (e, fields) in table {
            assert_eq!(enc(&e), format!("\x1b[{fields}_"), "{e:?}");
        }
    }

    #[test]
    fn ctrl_puts_the_control_character_in_unicode_char() {
        assert_eq!(
            enc(&event(Key::Char('a'), CTRL, "a")),
            "\x1b[65;30;1;1;8;1_"
        );
        assert_eq!(
            enc(&event(Key::Char('['), CTRL, "[")),
            "\x1b[219;26;27;1;8;1_"
        );
        assert_eq!(enc(&event(Key::Char('1'), CTRL, "1")), "\x1b[49;2;0;1;8;1_");
    }

    #[test]
    fn c0_keys_carry_their_byte() {
        let table = [
            (Key::Enter, "13;28;13"),
            (Key::Escape, "27;1;27"),
            (Key::Tab, "9;15;9"),
            (Key::Backspace, "8;14;8"),
            (Key::Keypad(KeypadKey::Enter), "13;28;13"),
        ];
        for (key, fields) in table {
            let enhanced = if key == Key::Keypad(KeypadKey::Enter) {
                256
            } else {
                0
            };
            let expected = format!("\x1b[{fields};1;{enhanced};1_");
            assert_eq!(enc(&event(key, NONE, "")), expected, "{key:?}");
        }
    }

    #[test]
    fn navigation_keys_are_enhanced_and_keypad_twins_are_not() {
        assert_eq!(enc(&event(Key::Up, NONE, "")), "\x1b[38;72;0;1;256;1_");
        assert_eq!(
            enc(&event(Key::Keypad(KeypadKey::Up), NONE, "")),
            "\x1b[38;72;0;1;0;1_"
        );
        assert_eq!(enc(&event(Key::Delete, SHIFT, "")), "\x1b[46;83;0;1;272;1_");
    }

    #[test]
    fn virtual_keys_and_scan_codes_follow_the_us_layout() {
        let table = [
            (Key::Char('1'), 0x31, 0x02),
            (Key::Char('='), 0xBB, 0x0D),
            (Key::Char('q'), 0x51, 0x10),
            (Key::Char(']'), 0xDD, 0x1B),
            (Key::Char('`'), 0xC0, 0x29),
            (Key::Char('\\'), 0xDC, 0x2B),
            (Key::Char('/'), 0xBF, 0x35),
            (Key::Char(' '), 0x20, 0x39),
            (Key::F(10), 0x79, 0x44),
            (Key::F(12), 0x7B, 0x58),
            (Key::F(13), 0x7C, 0x64),
            (Key::F(24), 0x87, 0x76),
            (Key::F(25), 0, 0),
            (Key::Keypad(KeypadKey::Digit7), 0x67, 0x47),
            (Key::Keypad(KeypadKey::Digit0), 0x60, 0x52),
            (Key::System(SystemKey::CapsLock), 0x14, 0x3A),
            (Key::Media(MediaKey::MuteVolume), 0xAD, 0x20),
        ];
        for (key, vk, scan) in table {
            let Win32Key(got_vk, got_scan, _) = win32_key(key);
            assert_eq!((got_vk, got_scan), (vk, scan), "{key:?}");
        }
    }

    #[test]
    fn lock_states_set_their_control_key_state_bits() {
        let locks = Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK;
        assert_eq!(
            enc(&event(Key::Char('a'), locks, "A")),
            "\x1b[65;30;65;1;160;1_"
        );
    }

    #[test]
    fn text_outside_the_bmp_goes_as_surrogate_records() {
        let e = event(Key::Char('a'), NONE, "😀");
        assert_eq!(enc(&e), "\x1b[65;30;55357;1;0;1_\x1b[65;30;56832;1;0;1_");
    }

    #[test]
    fn alt_keeps_the_typed_character() {
        assert_eq!(
            enc(&event(Key::Char('a'), ALT, "a")),
            "\x1b[65;30;97;1;2;1_"
        );
    }
}
