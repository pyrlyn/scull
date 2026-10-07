//! xterm's legacy key encoding: text as UTF-8, Ctrl as C0 bytes, Alt as an
//! ESC prefix, function keys as `CSI`/`SS3` with xterm's modifier parameter,
//! application cursor and keypad modes, and modifyOtherKeys. Separate from
//! the kitty encoder because it is what every program understands and the
//! terminal's default.
//!
//! The kitty specification's legacy section pins down the cases xterm leaves
//! to its resources; it is followed where the two agree, and xterm where
//! they differ (F3 is `SS3 R`):
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#legacy-key-event-encoding>,
//! <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys>.

use crate::bytes::{CSI, DECIMAL_RADIX, ESC, SEMI, SS3, push_char, push_decimal};
use crate::key::{
    BS, CR, DEL, FINAL_TILDE, FINAL_U, Key, KeyAction, KeyEvent, KeypadKey, LETTER_KEY_NUMBER,
    Modifiers, SystemKey, TAB, ctrl_byte, functional,
};
use crate::keyboard::{KeyModes, ModifyOtherKeys};

/// modifyOtherKeys reports are `CSI 27 ; mods ; char ~`.
const MODIFY_OTHER_KEYS_NUMBER: u32 = 27;
/// Shift+Tab is back-tab, `CSI Z`.
const BACKTAB: u8 = b'Z';
/// xterm keeps F3 as `SS3 R`; kitty moved it to `13 ~` only because
/// `CSI 1 ; m R` collides with the cursor position report.
const F3_FINAL: u8 = b'R';
/// F1–F4 finals; unmodified they always go out as `SS3`.
const PF1: u8 = b'P';
const PF4: u8 = b'S';
/// Menu is terminfo `kf16`, `CSI 29 ~`.
const MENU_NUMBER: u32 = 29;
/// DECKPAM finals: digits are `SS3 p`..`SS3 y`.
const KEYPAD_DIGIT0_FINAL: u8 = b'p';

pub(crate) fn encode(event: &KeyEvent<'_>, modes: &KeyModes, out: &mut Vec<u8>) {
    // Legacy encodings have no key-up.
    if event.action == KeyAction::Release {
        return;
    }
    let mods = event.reported_mods() - Modifiers::LOCKS;
    match event.key {
        Key::Char(base) => text_key(event, base, mods, modes, out),
        Key::Enter => c0_key(Key::Enter, CR, mods, modes, out),
        Key::Escape => c0_key(Key::Escape, ESC, mods, modes, out),
        Key::Tab => c0_key(Key::Tab, TAB, mods, modes, out),
        Key::Backspace => c0_key(Key::Backspace, DEL, mods, modes, out),
        Key::Keypad(k) => keypad(event, k, mods, modes, out),
        Key::F(3) => function_key((LETTER_KEY_NUMBER, F3_FINAL), mods, modes, out),
        Key::System(SystemKey::Menu) => function_key((MENU_NUMBER, FINAL_TILDE), mods, modes, out),
        key => {
            if let Some(form) = functional(key) {
                function_key(form, mods, modes, out);
            }
        }
    }
}

fn function_key((number, fin): (u32, u8), mods: Modifiers, modes: &KeyModes, out: &mut Vec<u8>) {
    // Media, modifier, lock and F13+ keys have no legacy spelling; sending
    // kitty's private-use codes would confuse programs that never asked.
    if fin == FINAL_U {
        return;
    }
    if fin != FINAL_TILDE && mods.is_empty() {
        let ss3 = modes.application_cursor || matches!(fin, PF1..=PF4);
        out.extend_from_slice(if ss3 { &SS3 } else { &CSI });
    } else {
        out.extend_from_slice(&CSI);
        push_decimal(out, number);
        if !mods.is_empty() {
            out.push(SEMI);
            push_decimal(out, mods.wire());
        }
    }
    out.push(fin);
}

/// Enter, Escape, Tab and Backspace, per the kitty specification's C0 table.
fn c0_key(key: Key, byte: u8, mods: Modifiers, modes: &KeyModes, out: &mut Vec<u8>) {
    let back_tab = key == Key::Tab && mods == Modifiers::SHIFT;
    if modes.modify_other_keys == ModifyOtherKeys::Level2 && !mods.is_empty() && !back_tab {
        return modify_other_keys(u32::from(byte), mods, out);
    }
    let all_three = Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT;
    if mods.intersects(Modifiers::BEYOND_XTERM) || mods.contains(all_three) {
        return fallback(u32::from(byte), u32::from(byte), mods, modes, out);
    }
    if mods.contains(Modifiers::ALT) {
        out.push(ESC);
    }
    match key {
        Key::Tab if mods.contains(Modifiers::SHIFT) => {
            out.extend_from_slice(&CSI);
            out.push(BACKTAB);
        }
        Key::Backspace if mods.contains(Modifiers::CTRL) => out.push(BS),
        _ => out.push(byte),
    }
}

/// Keys that type text, per the kitty specification's legacy text keys.
fn text_key(
    event: &KeyEvent<'_>,
    base: char,
    mods: Modifiers,
    modes: &KeyModes,
    out: &mut Vec<u8>,
) {
    let code = event.unshifted.unwrap_or(base);
    // Shift does not change what Space sends (Ctrl+Shift+Space is NUL).
    let mods = if base == ' ' {
        mods - Modifiers::SHIFT
    } else {
        mods
    };
    let typed = event.single_char().unwrap_or(code);
    if mods.difference(Modifiers::SHIFT).is_empty() {
        if event.has_text() {
            event.clean_text().for_each(|c| push_char(out, c));
        } else if mods.is_empty() {
            push_char(out, code);
        }
        return;
    }
    if modes.modify_other_keys == ModifyOtherKeys::Level2 {
        return modify_other_keys(u32::from(typed), mods, out);
    }
    if mods.intersects(Modifiers::BEYOND_XTERM) || mods.contains(Modifiers::CTRL | Modifiers::SHIFT)
    {
        return fallback(u32::from(code), u32::from(typed), mods, modes, out);
    }
    if mods.contains(Modifiers::ALT) {
        out.push(ESC);
    }
    if mods.contains(Modifiers::CTRL) {
        // On a non-Latin layout Ctrl acts on the US key, so Ctrl+С is ^C.
        let ascii = if code.is_ascii() { code } else { base };
        match ctrl_byte(ascii) {
            Some(byte) => out.push(byte),
            None => push_char(out, ascii),
        }
    } else if event.has_text() {
        event.clean_text().for_each(|c| push_char(out, c));
    } else {
        push_char(out, code);
    }
}

fn keypad(
    event: &KeyEvent<'_>,
    k: KeypadKey,
    mods: Modifiers,
    modes: &KeyModes,
    out: &mut Vec<u8>,
) {
    use KeypadKey as K;
    // Legacy reports keypad keys as their main-block twins.
    let twin = match k {
        K::Left => Some(Key::Left),
        K::Right => Some(Key::Right),
        K::Up => Some(Key::Up),
        K::Down => Some(Key::Down),
        K::PageUp => Some(Key::PageUp),
        K::PageDown => Some(Key::PageDown),
        K::Home => Some(Key::Home),
        K::End => Some(Key::End),
        K::Insert => Some(Key::Insert),
        K::Delete => Some(Key::Delete),
        K::Begin => Some(Key::Keypad(K::Begin)),
        _ => None,
    };
    if let Some(twin) = twin {
        if let Some(form) = functional(twin) {
            function_key(form, mods, modes, out);
        }
        return;
    }
    if modes.application_keypad
        && mods.is_empty()
        && let Some(fin) = application_keypad_final(k)
    {
        out.extend_from_slice(&SS3);
        out.push(fin);
        return;
    }
    if k == K::Enter {
        c0_key(Key::Enter, CR, mods, modes, out);
    } else if let Some(c) = k.text_char() {
        text_key(event, c, mods, modes, out);
    }
}

/// DECKPAM finals from xterm's VT220-style keypad table.
fn application_keypad_final(k: KeypadKey) -> Option<u8> {
    use KeypadKey as K;
    Some(match k {
        K::Decimal => b'n',
        K::Divide => b'o',
        K::Multiply => b'j',
        K::Subtract => b'm',
        K::Add => b'k',
        K::Enter => b'M',
        K::Equal => b'X',
        K::Separator => b'l',
        _ => {
            let digit = k.text_char()?.to_digit(DECIMAL_RADIX)?;
            KEYPAD_DIGIT0_FINAL + u8::try_from(digit).ok()?
        }
    })
}

/// A combination legacy cannot spell: modifyOtherKeys when enabled, else
/// kitty's `CSI code ; mods u`, which kitty sends even in legacy mode.
fn fallback(code: u32, typed: u32, mods: Modifiers, modes: &KeyModes, out: &mut Vec<u8>) {
    if modes.modify_other_keys != ModifyOtherKeys::Off {
        return modify_other_keys(typed, mods, out);
    }
    out.extend_from_slice(&CSI);
    push_decimal(out, code);
    out.push(SEMI);
    push_decimal(out, mods.wire());
    out.push(FINAL_U);
}

fn modify_other_keys(typed: u32, mods: Modifiers, out: &mut Vec<u8>) {
    out.extend_from_slice(&CSI);
    push_decimal(out, MODIFY_OTHER_KEYS_NUMBER);
    out.push(SEMI);
    push_decimal(out, mods.wire());
    out.push(SEMI);
    push_decimal(out, typed);
    out.push(FINAL_TILDE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::tests::{ALT, ALT_SHIFT, CTRL, CTRL_ALT, CTRL_SHIFT, NONE, SHIFT, event};
    use crate::key::{MediaKey, ModifierKey};

    const CSI_S: &str = "\x1b[";
    const SS3_S: &str = "\x1bO";

    fn encode_with(event: &KeyEvent<'_>, modes: &KeyModes) -> String {
        let mut out = Vec::new();
        encode(event, modes, &mut out);
        String::from_utf8(out).unwrap()
    }

    fn enc(key: Key, mods: Modifiers, text: &str) -> String {
        encode_with(&event(key, mods, text), &KeyModes::default())
    }

    fn modes(modify_other_keys: ModifyOtherKeys) -> KeyModes {
        KeyModes {
            modify_other_keys,
            ..KeyModes::default()
        }
    }

    /// Columns of the specification's C0 table, in its order.
    const COLUMNS: [Modifiers; 7] = [NONE, CTRL, ALT, SHIFT, CTRL_SHIFT, ALT_SHIFT, CTRL_ALT];

    #[test]
    fn c0_keys_match_the_kitty_legacy_table() {
        let table: [(Key, [&str; 7]); 5] = [
            (
                Key::Enter,
                ["\r", "\r", "\x1b\r", "\r", "\r", "\x1b\r", "\x1b\r"],
            ),
            (
                Key::Escape,
                [
                    "\x1b", "\x1b", "\x1b\x1b", "\x1b", "\x1b", "\x1b\x1b", "\x1b\x1b",
                ],
            ),
            (
                Key::Backspace,
                [
                    "\x7f", "\x08", "\x1b\x7f", "\x7f", "\x08", "\x1b\x7f", "\x1b\x08",
                ],
            ),
            (
                Key::Tab,
                [
                    "\t",
                    "\t",
                    "\x1b\t",
                    "\x1b[Z",
                    "\x1b[Z",
                    "\x1b\x1b[Z",
                    "\x1b\t",
                ],
            ),
            (
                Key::Char(' '),
                [" ", "\0", "\x1b ", " ", "\0", "\x1b ", "\x1b\0"],
            ),
        ];
        for (key, row) in table {
            let text = if key == Key::Char(' ') { " " } else { "" };
            for (mods, expected) in COLUMNS.into_iter().zip(row) {
                assert_eq!(enc(key, mods, text), expected, "{key:?} {mods:?}");
            }
        }
    }

    #[test]
    fn text_keys_match_the_kitty_example_encodings() {
        // Columns: plain, shift, alt, ctrl, shift+alt, alt+ctrl, ctrl+shift.
        let columns = [NONE, SHIFT, ALT, CTRL, ALT_SHIFT, CTRL_ALT, CTRL_SHIFT];
        let table = [
            (
                'i',
                'I',
                ["i", "I", "\x1bi", "\t", "\x1bI", "\x1b\t", "\x1b[105;6u"],
            ),
            (
                '3',
                '#',
                ["3", "#", "\x1b3", "\x1b", "\x1b#", "\x1b\x1b", "\x1b[51;6u"],
            ),
            (
                ';',
                ':',
                [";", ":", "\x1b;", ";", "\x1b:", "\x1b;", "\x1b[59;6u"],
            ),
        ];
        for (key, shifted, row) in table {
            for (mods, expected) in columns.into_iter().zip(row) {
                let typed = if mods.contains(SHIFT) { shifted } else { key };
                let text = typed.to_string();
                assert_eq!(enc(Key::Char(key), mods, &text), expected, "{key} {mods:?}");
            }
        }
    }

    #[test]
    fn functional_keys_match_the_kitty_legacy_table() {
        // (key, normal, application cursor mode, with shift)
        let table = [
            (Key::Insert, "[2~", "[2~", "[2;2~"),
            (Key::Delete, "[3~", "[3~", "[3;2~"),
            (Key::PageUp, "[5~", "[5~", "[5;2~"),
            (Key::PageDown, "[6~", "[6~", "[6;2~"),
            (Key::Up, "[A", "OA", "[1;2A"),
            (Key::Down, "[B", "OB", "[1;2B"),
            (Key::Right, "[C", "OC", "[1;2C"),
            (Key::Left, "[D", "OD", "[1;2D"),
            (Key::Home, "[H", "OH", "[1;2H"),
            (Key::End, "[F", "OF", "[1;2F"),
            (Key::F(1), "OP", "OP", "[1;2P"),
            (Key::F(2), "OQ", "OQ", "[1;2Q"),
            (Key::F(3), "OR", "OR", "[1;2R"),
            (Key::F(4), "OS", "OS", "[1;2S"),
            (Key::F(5), "[15~", "[15~", "[15;2~"),
            (Key::F(6), "[17~", "[17~", "[17;2~"),
            (Key::F(7), "[18~", "[18~", "[18;2~"),
            (Key::F(8), "[19~", "[19~", "[19;2~"),
            (Key::F(9), "[20~", "[20~", "[20;2~"),
            (Key::F(10), "[21~", "[21~", "[21;2~"),
            (Key::F(11), "[23~", "[23~", "[23;2~"),
            (Key::F(12), "[24~", "[24~", "[24;2~"),
            (Key::System(SystemKey::Menu), "[29~", "[29~", "[29;2~"),
        ];
        let app = KeyModes {
            application_cursor: true,
            ..KeyModes::default()
        };
        for (key, normal, application, shifted) in table {
            let plain = event(key, NONE, "");
            let default = KeyModes::default();
            assert_eq!(
                encode_with(&plain, &default),
                format!("\x1b{normal}"),
                "{key:?}"
            );
            assert_eq!(
                encode_with(&plain, &app),
                format!("\x1b{application}"),
                "{key:?}"
            );
            assert_eq!(enc(key, SHIFT, ""), format!("\x1b{shifted}"), "{key:?}");
        }
    }

    #[test]
    fn modifier_parameter_is_one_plus_the_bits() {
        let table = [
            (ALT, "3"),
            (CTRL, "5"),
            (CTRL_SHIFT, "6"),
            (CTRL_ALT, "7"),
            (Modifiers::SUPER, "9"),
        ];
        for (mods, param) in table {
            assert_eq!(enc(Key::Up, mods, ""), format!("{CSI_S}1;{param}A"));
            assert_eq!(enc(Key::F(5), mods, ""), format!("{CSI_S}15;{param}~"));
        }
    }

    #[test]
    fn lock_states_are_not_encoded() {
        assert_eq!(enc(Key::Up, Modifiers::LOCKS, ""), format!("{CSI_S}A"));
        assert_eq!(enc(Key::Char('a'), Modifiers::CAPS_LOCK, "A"), "A");
        assert_eq!(enc(Key::Char('a'), Modifiers::NUM_LOCK | CTRL, "a"), "\x01");
    }

    #[test]
    fn releases_and_keys_without_a_legacy_spelling_send_nothing() {
        let mut release = event(Key::Char('a'), NONE, "a");
        release.action = KeyAction::Release;
        assert_eq!(encode_with(&release, &KeyModes::default()), "");
        for key in [
            Key::F(13),
            Key::F(0),
            Key::Media(MediaKey::Play),
            Key::Modifier(ModifierKey::LeftShift),
            Key::System(SystemKey::CapsLock),
        ] {
            assert_eq!(enc(key, NONE, ""), "", "{key:?}");
        }
    }

    #[test]
    fn repeat_encodes_like_press() {
        let mut repeat = event(Key::Up, NONE, "");
        repeat.action = KeyAction::Repeat;
        assert_eq!(
            encode_with(&repeat, &KeyModes::default()),
            format!("{CSI_S}A")
        );
    }

    #[test]
    fn keypad_follows_numeric_and_application_modes() {
        let app = KeyModes {
            application_keypad: true,
            ..KeyModes::default()
        };
        let table = [
            (KeypadKey::Digit0, "0", "p"),
            (KeypadKey::Digit1, "1", "q"),
            (KeypadKey::Digit9, "9", "y"),
            (KeypadKey::Decimal, ".", "n"),
            (KeypadKey::Divide, "/", "o"),
            (KeypadKey::Multiply, "*", "j"),
            (KeypadKey::Subtract, "-", "m"),
            (KeypadKey::Add, "+", "k"),
            (KeypadKey::Equal, "=", "X"),
            (KeypadKey::Separator, ",", "l"),
        ];
        for (k, text, fin) in table {
            let plain = event(Key::Keypad(k), NONE, text);
            assert_eq!(encode_with(&plain, &KeyModes::default()), text, "{k:?}");
            assert_eq!(encode_with(&plain, &app), format!("{SS3_S}{fin}"), "{k:?}");
            // Modified keypad keys fall back to what the key types.
            let modified = event(Key::Keypad(k), CTRL_SHIFT, text);
            assert!(encode_with(&modified, &app).ends_with('u'), "{k:?}");
        }
        let enter = event(Key::Keypad(KeypadKey::Enter), NONE, "");
        assert_eq!(encode_with(&enter, &KeyModes::default()), "\r");
        assert_eq!(encode_with(&enter, &app), format!("{SS3_S}M"));
    }

    #[test]
    fn keypad_navigation_reports_as_main_block_keys() {
        let app_cursor = KeyModes {
            application_cursor: true,
            ..KeyModes::default()
        };
        assert_eq!(
            enc(Key::Keypad(KeypadKey::Left), NONE, ""),
            format!("{CSI_S}D")
        );
        assert_eq!(
            enc(Key::Keypad(KeypadKey::Delete), NONE, ""),
            format!("{CSI_S}3~")
        );
        assert_eq!(
            enc(Key::Keypad(KeypadKey::Begin), NONE, ""),
            format!("{CSI_S}E")
        );
        let left = event(Key::Keypad(KeypadKey::Left), NONE, "");
        assert_eq!(encode_with(&left, &app_cursor), format!("{SS3_S}D"));
    }

    #[test]
    fn unencodable_combinations_fall_back_to_csi_u() {
        assert_eq!(
            enc(Key::Char('a'), Modifiers::SUPER, "a"),
            format!("{CSI_S}97;9u")
        );
        assert_eq!(
            enc(Key::Enter, Modifiers::SUPER, ""),
            format!("{CSI_S}13;9u")
        );
        let all = CTRL_ALT | SHIFT;
        assert_eq!(enc(Key::Backspace, all, ""), format!("{CSI_S}127;8u"));
    }

    #[test]
    fn modify_other_keys_level_two_reports_every_modified_key() {
        let level2 = modes(ModifyOtherKeys::Level2);
        let table = [
            (Key::Char('a'), CTRL, "a", "27;5;97~"),
            (Key::Char('a'), ALT, "a", "27;3;97~"),
            (Key::Char('a'), CTRL_SHIFT, "A", "27;6;65~"),
            (Key::Char('1'), CTRL, "1", "27;5;49~"),
            (Key::Enter, CTRL, "", "27;5;13~"),
            (Key::Escape, ALT, "", "27;3;27~"),
            (Key::Tab, CTRL, "", "27;5;9~"),
            (Key::Backspace, CTRL, "", "27;5;127~"),
        ];
        for (key, mods, text, expected) in table {
            let e = event(key, mods, text);
            let got = encode_with(&e, &level2);
            assert_eq!(got, format!("{CSI_S}{expected}"), "{key:?} {mods:?}");
        }
        // Shift alone still types text, and Shift+Tab stays back-tab.
        assert_eq!(
            encode_with(&event(Key::Char('a'), SHIFT, "A"), &level2),
            "A"
        );
        assert_eq!(
            encode_with(&event(Key::Tab, SHIFT, ""), &level2),
            format!("{CSI_S}Z")
        );
        // Function keys keep their own modifier parameter.
        assert_eq!(
            encode_with(&event(Key::Up, CTRL, ""), &level2),
            format!("{CSI_S}1;5A")
        );
    }

    #[test]
    fn modify_other_keys_level_one_reports_only_what_legacy_cannot() {
        let level1 = modes(ModifyOtherKeys::Level1);
        assert_eq!(
            encode_with(&event(Key::Char('a'), CTRL, "a"), &level1),
            "\x01"
        );
        assert_eq!(
            encode_with(&event(Key::Char('a'), ALT, "a"), &level1),
            "\x1ba"
        );
        let ctrl_shift = event(Key::Char('a'), CTRL_SHIFT, "A");
        assert_eq!(
            encode_with(&ctrl_shift, &level1),
            format!("{CSI_S}27;6;65~")
        );
    }

    #[test]
    fn ctrl_acts_on_the_us_key_on_other_layouts() {
        let mut e = event(Key::Char('c'), CTRL, "с");
        e.unshifted = Some('с');
        assert_eq!(encode_with(&e, &KeyModes::default()), "\x03");
        e.mods = NONE;
        assert_eq!(encode_with(&e, &KeyModes::default()), "с");
        e.mods = ALT;
        assert_eq!(encode_with(&e, &KeyModes::default()), "\x1bс");
    }

    #[test]
    fn modifiers_the_layout_consumed_are_not_encoded() {
        let mut e = event(Key::Char('a'), ALT, "å");
        e.consumed = ALT;
        assert_eq!(encode_with(&e, &KeyModes::default()), "å");
    }

    #[test]
    fn control_characters_in_text_never_reach_the_program() {
        assert_eq!(enc(Key::Char('a'), NONE, "\x1b[201~a"), "[201~a");
        assert_eq!(enc(Key::Char('a'), ALT, "\u{9b}a"), "\x1ba");
    }
}
