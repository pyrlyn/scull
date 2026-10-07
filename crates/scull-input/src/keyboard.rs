//! The one entry point for key events: picks win32-input-mode, the kitty
//! protocol or the legacy xterm encoding from the modes the terminal holds.
//! Separate so the precedence between the three lives in one place.

use crate::key::KeyEvent;
use crate::kitty::{self, KittyFlags};
use crate::{legacy, win32};

/// xterm's `modifyOtherKeys` resource, set by `CSI > 4 ; level m`.
/// <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html> (XTMODKEYS)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ModifyOtherKeys {
    /// Off: unencodable combinations fall back to kitty's `CSI u` form.
    #[default]
    Off,
    /// Level 1: only combinations with no legacy spelling become
    /// `CSI 27 ; mods ; char ~`.
    Level1,
    /// Level 2: every modified key except Shift alone on text.
    Level2,
}

/// The terminal modes key encoding depends on. The terminal owns them;
/// this crate only reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyModes {
    /// DECCKM (`CSI ? 1 h`): unmodified cursor keys send `SS3`.
    pub application_cursor: bool,
    /// DECKPAM (`ESC =`): the keypad sends `SS3` instead of digits.
    pub application_keypad: bool,
    /// xterm `modifyOtherKeys`.
    pub modify_other_keys: ModifyOtherKeys,
    /// The kitty flags of the active screen's stack.
    pub kitty: KittyFlags,
    /// win32-input-mode (`CSI ? 9001 h`), which ConPTY sets.
    pub win32_input: bool,
}

/// Appends the bytes the program expects for `event`. Nothing is appended
/// when the event has no encoding in the active modes (a release in legacy
/// mode, a key being composed, an F-key beyond F35).
pub fn encode_key(event: &KeyEvent<'_>, modes: &KeyModes, out: &mut Vec<u8>) {
    if event.composing {
        return;
    }
    // ConPTY asks for win32 records to rebuild console input records; any
    // other format would lose key-up events and scan codes.
    if modes.win32_input {
        win32::encode(event, out);
    } else if modes.kitty.is_legacy() {
        legacy::encode(event, modes, out);
    } else {
        kitty::encode(event, modes.kitty, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Key, KeyAction};

    fn encode(event: &KeyEvent<'_>, modes: &KeyModes) -> Vec<u8> {
        let mut out = Vec::new();
        encode_key(event, modes, &mut out);
        out
    }

    #[test]
    fn composing_events_send_nothing_in_any_mode() {
        let mut event = KeyEvent::new(Key::Char('a'));
        event.text = "a";
        event.composing = true;
        let kitty = KeyModes {
            kitty: KittyFlags::all(),
            ..KeyModes::default()
        };
        let win32 = KeyModes {
            win32_input: true,
            ..KeyModes::default()
        };
        for modes in [KeyModes::default(), kitty, win32] {
            assert!(encode(&event, &modes).is_empty(), "{modes:?}");
        }
    }

    #[test]
    fn win32_input_mode_wins_over_kitty_flags() {
        let mut event = KeyEvent::new(Key::Char('a'));
        event.text = "a";
        let modes = KeyModes {
            win32_input: true,
            kitty: KittyFlags::all(),
            ..KeyModes::default()
        };
        assert_eq!(encode(&event, &modes), b"\x1b[65;30;97;1;0;1_");
    }

    #[test]
    fn alternates_and_text_flags_alone_keep_legacy_encoding() {
        let mut event = KeyEvent::new(Key::Escape);
        event.action = KeyAction::Press;
        let modes = KeyModes {
            kitty: KittyFlags::REPORT_ALTERNATES | KittyFlags::REPORT_TEXT,
            ..KeyModes::default()
        };
        assert_eq!(encode(&event, &modes), b"\x1b");
    }

    mod properties {
        use proptest::prelude::*;

        use super::super::*;
        use crate::key::{Key, KeyAction, KeypadKey, MediaKey, ModifierKey, Modifiers, SystemKey};

        const KEYPAD: [KeypadKey; 4] = [
            KeypadKey::Digit5,
            KeypadKey::Enter,
            KeypadKey::Begin,
            KeypadKey::Separator,
        ];

        fn key() -> impl Strategy<Value = Key> {
            prop_oneof![
                any::<char>().prop_map(Key::Char),
                any::<u8>().prop_map(Key::F),
                prop::sample::select(KEYPAD.to_vec()).prop_map(Key::Keypad),
                prop::sample::select(vec![
                    Key::Escape,
                    Key::Enter,
                    Key::Tab,
                    Key::Backspace,
                    Key::Up,
                    Key::Home,
                    Key::Delete,
                    Key::Media(MediaKey::Record),
                    Key::Modifier(ModifierKey::RightMeta),
                    Key::System(SystemKey::Menu),
                    Key::System(SystemKey::NumLock),
                ]),
            ]
        }

        fn action() -> impl Strategy<Value = KeyAction> {
            prop::sample::select(vec![
                KeyAction::Press,
                KeyAction::Repeat,
                KeyAction::Release,
            ])
        }

        fn modify_other_keys() -> impl Strategy<Value = ModifyOtherKeys> {
            prop::sample::select(vec![
                ModifyOtherKeys::Off,
                ModifyOtherKeys::Level1,
                ModifyOtherKeys::Level2,
            ])
        }

        proptest! {
            #[test]
            fn no_key_event_panics_in_any_mode(
                key in key(),
                action in action(),
                mods in any::<u8>(),
                consumed in any::<u8>(),
                text in ".{0,4}",
                unshifted in any::<Option<char>>(),
                flags in any::<u16>(),
                cursor in any::<bool>(),
                keypad in any::<bool>(),
                win32_input in any::<bool>(),
                mok in modify_other_keys(),
            ) {
                let event = KeyEvent {
                    key,
                    action,
                    mods: Modifiers::from_bits_retain(mods),
                    consumed: Modifiers::from_bits_retain(consumed),
                    text: &text,
                    unshifted,
                    composing: false,
                };
                let modes = KeyModes {
                    application_cursor: cursor,
                    application_keypad: keypad,
                    modify_other_keys: mok,
                    kitty: KittyFlags::from_param(flags),
                    win32_input,
                };
                let mut out = Vec::new();
                encode_key(&event, &modes, &mut out);
            }

            #[test]
            fn flag_stack_never_exceeds_its_cap(
                ops in prop::collection::vec((0_u8..4, any::<u16>(), any::<u16>()), 0..64),
            ) {
                let mut stack = crate::kitty::KittyFlagStack::default();
                for (op, a, b) in ops {
                    match op {
                        0 => stack.push(a),
                        1 => stack.pop(a),
                        2 => stack.set(a, b),
                        _ => stack.clear(),
                    }
                    prop_assert!(stack.depth() <= crate::kitty::KITTY_FLAG_STACK_DEPTH);
                    prop_assert!(KittyFlags::all().contains(stack.current()));
                }
            }
        }
    }
}
