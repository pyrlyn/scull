//! Input from the host: key, text, paste, focus and mouse events, encoded
//! by the core against the modes the program set and handed to the child,
//! and scrolling the viewport through history. The host never writes an
//! escape sequence itself (`docs/research/ffi-native-ui.md` §4.5).
//! Separate from `spawn.rs` because this is the event vocabulary, not the
//! byte pipe.
//!
//! Key codes: a key of the main block is the code point it types on the
//! US layout with no modifiers (`'a'`, `'1'`, `' '`), as scull-input's
//! `Key::Char`; every other key has a code from 57344 up. From Caps Lock
//! on, those are the codes of the kitty keyboard protocol's functional key
//! table (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>), so one
//! number names each key on both sides; Escape to End and F1 to F12, which
//! the protocol spells with legacy numbers, take the codes just below and
//! between them.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use scull_input::{
    Key, KeyAction, KeyEvent, KeypadKey, MediaKey, ModifierKey, Modifiers, MouseAction,
    MouseButton, MouseEvent, SystemKey,
};
use scull_pty::PtyError;
use scull_term::Terminal;

use crate::guard::{SizedStruct, read_sized, tt_status, with_term};
use crate::spawn::{text, tt_str};
use crate::term::tt_term;

/// `tt_key_event.action`: the key went down.
pub const TT_KEY_PRESS: u8 = 1;
/// The key auto-repeats.
pub const TT_KEY_REPEAT: u8 = 2;
/// The key went up.
pub const TT_KEY_RELEASE: u8 = 3;

/// `tt_key_event.mods` and `tt_mouse_event.mods` bits.
pub const TT_MOD_SHIFT: u8 = 1;
/// Alt, Option on macOS.
pub const TT_MOD_ALT: u8 = 2;
/// Control.
pub const TT_MOD_CTRL: u8 = 4;
/// Super: Command on macOS, the Windows key.
pub const TT_MOD_SUPER: u8 = 8;
/// Hyper.
pub const TT_MOD_HYPER: u8 = 16;
/// Meta.
pub const TT_MOD_META: u8 = 32;
/// Caps Lock is on.
pub const TT_MOD_CAPS_LOCK: u8 = 64;
/// Num Lock is on.
pub const TT_MOD_NUM_LOCK: u8 = 128;

const _: () = assert!(
    TT_MOD_SHIFT == Modifiers::SHIFT.bits()
        && TT_MOD_ALT == Modifiers::ALT.bits()
        && TT_MOD_CTRL == Modifiers::CTRL.bits()
        && TT_MOD_SUPER == Modifiers::SUPER.bits()
        && TT_MOD_HYPER == Modifiers::HYPER.bits()
        && TT_MOD_META == Modifiers::META.bits()
        && TT_MOD_CAPS_LOCK == Modifiers::CAPS_LOCK.bits()
        && TT_MOD_NUM_LOCK == Modifiers::NUM_LOCK.bits()
);

/// `tt_key_event.key`: Escape. The keys after it, one code each, are
/// Enter, Tab, Backspace, Insert, Delete, Left, Right, Up, Down, Page Up,
/// Page Down, Home and End (`TT_KEY_END`).
pub const TT_KEY_ESCAPE: u32 = 57_344;
/// The last of the keys from `TT_KEY_ESCAPE`.
pub const TT_KEY_END: u32 = 57_357;
/// Caps Lock; then Scroll Lock, Num Lock, Print Screen, Pause, Menu.
pub const TT_KEY_CAPS_LOCK: u32 = 57_358;
/// F1; F`n` is `TT_KEY_F1 + n - 1` up to F35.
pub const TT_KEY_F1: u32 = 57_364;
/// Keypad 0; then 1 to 9, Decimal, Divide, Multiply, Subtract, Add, Enter,
/// Equal, Separator, Left, Right, Up, Down, Page Up, Page Down, Home, End,
/// Insert, Delete, Begin.
pub const TT_KEY_KP_0: u32 = 57_399;
/// Play; then Pause, Play/Pause, Reverse, Stop, Fast Forward, Rewind, Next
/// Track, Previous Track, Record, Volume Down, Volume Up, Mute.
pub const TT_KEY_MEDIA_PLAY: u32 = 57_428;
/// Left Shift on its own; then Left Control, Left Alt, Left Super, Left
/// Hyper, Left Meta, the same six on the right, ISO Level 3 Shift and ISO
/// Level 5 Shift.
pub const TT_KEY_LEFT_SHIFT: u32 = 57_441;

const _: () = assert!(
    TT_KEY_END == TT_KEY_ESCAPE + NAMED.len() as u32 - 1
        && TT_KEY_CAPS_LOCK == SystemKey::CapsLock as u32
        && TT_KEY_KP_0 == KeypadKey::Digit0 as u32
        && TT_KEY_MEDIA_PLAY == MediaKey::Play as u32
        && TT_KEY_LEFT_SHIFT == ModifierKey::LeftShift as u32
        && TT_KEY_F1 + F13_OFFSET == KITTY_F13
);

/// F13 counted from F1.
const F13_OFFSET: u32 = 12;
/// kitty's code for F13, the first function key it numbers; F1 to F12 sit
/// just below it.
const KITTY_F13: u32 = 57_376;
/// Function keys the encoders know.
const LAST_F_KEY: u32 = 35;

/// The keys from `TT_KEY_ESCAPE`, in code order.
const NAMED: [Key; 14] = [
    Key::Escape,
    Key::Enter,
    Key::Tab,
    Key::Backspace,
    Key::Insert,
    Key::Delete,
    Key::Left,
    Key::Right,
    Key::Up,
    Key::Down,
    Key::PageUp,
    Key::PageDown,
    Key::Home,
    Key::End,
];

/// Every key whose scull-input discriminant is its kitty code.
const CODED: [Key; 62] = {
    use KeypadKey as K;
    use MediaKey as Md;
    use ModifierKey as Mo;
    use SystemKey as S;
    [
        Key::System(S::CapsLock),
        Key::System(S::ScrollLock),
        Key::System(S::NumLock),
        Key::System(S::PrintScreen),
        Key::System(S::Pause),
        Key::System(S::Menu),
        Key::Keypad(K::Digit0),
        Key::Keypad(K::Digit1),
        Key::Keypad(K::Digit2),
        Key::Keypad(K::Digit3),
        Key::Keypad(K::Digit4),
        Key::Keypad(K::Digit5),
        Key::Keypad(K::Digit6),
        Key::Keypad(K::Digit7),
        Key::Keypad(K::Digit8),
        Key::Keypad(K::Digit9),
        Key::Keypad(K::Decimal),
        Key::Keypad(K::Divide),
        Key::Keypad(K::Multiply),
        Key::Keypad(K::Subtract),
        Key::Keypad(K::Add),
        Key::Keypad(K::Enter),
        Key::Keypad(K::Equal),
        Key::Keypad(K::Separator),
        Key::Keypad(K::Left),
        Key::Keypad(K::Right),
        Key::Keypad(K::Up),
        Key::Keypad(K::Down),
        Key::Keypad(K::PageUp),
        Key::Keypad(K::PageDown),
        Key::Keypad(K::Home),
        Key::Keypad(K::End),
        Key::Keypad(K::Insert),
        Key::Keypad(K::Delete),
        Key::Keypad(K::Begin),
        Key::Media(Md::Play),
        Key::Media(Md::Pause),
        Key::Media(Md::PlayPause),
        Key::Media(Md::Reverse),
        Key::Media(Md::Stop),
        Key::Media(Md::FastForward),
        Key::Media(Md::Rewind),
        Key::Media(Md::TrackNext),
        Key::Media(Md::TrackPrevious),
        Key::Media(Md::Record),
        Key::Media(Md::LowerVolume),
        Key::Media(Md::RaiseVolume),
        Key::Media(Md::MuteVolume),
        Key::Modifier(Mo::LeftShift),
        Key::Modifier(Mo::LeftControl),
        Key::Modifier(Mo::LeftAlt),
        Key::Modifier(Mo::LeftSuper),
        Key::Modifier(Mo::LeftHyper),
        Key::Modifier(Mo::LeftMeta),
        Key::Modifier(Mo::RightShift),
        Key::Modifier(Mo::RightControl),
        Key::Modifier(Mo::RightAlt),
        Key::Modifier(Mo::RightSuper),
        Key::Modifier(Mo::RightHyper),
        Key::Modifier(Mo::RightMeta),
        Key::Modifier(Mo::IsoLevel3Shift),
        Key::Modifier(Mo::IsoLevel5Shift),
    ]
};

/// `tt_mouse_event.action`: a button went down; wheel steps are presses.
pub const TT_MOUSE_PRESS: u8 = 1;
/// A button went up.
pub const TT_MOUSE_RELEASE: u8 = 2;
/// The pointer moved to another cell, or pixel while the program asks
/// for pixels; send it only then.
pub const TT_MOUSE_MOTION: u8 = 3;

/// `tt_mouse_event.button`: none, for motion with no button held. Others
/// are X11's numbers: 1 left, 2 middle, 3 right, 4 to 7 wheel up, down,
/// left and right, 8 back, 9 forward, 10 and 11.
pub const TT_MOUSE_NONE: u8 = 0;
/// The left button.
pub const TT_MOUSE_LEFT: u8 = 1;
/// The middle button.
pub const TT_MOUSE_MIDDLE: u8 = 2;
/// The right button.
pub const TT_MOUSE_RIGHT: u8 = 3;
/// One wheel step up.
pub const TT_MOUSE_WHEEL_UP: u8 = 4;
/// One wheel step down.
pub const TT_MOUSE_WHEEL_DOWN: u8 = 5;
/// One wheel step left.
pub const TT_MOUSE_WHEEL_LEFT: u8 = 6;
/// One wheel step right.
pub const TT_MOUSE_WHEEL_RIGHT: u8 = 7;

/// Buttons by X11 number, from 1.
const BUTTONS: [MouseButton; 11] = [
    MouseButton::Left,
    MouseButton::Middle,
    MouseButton::Right,
    MouseButton::WheelUp,
    MouseButton::WheelDown,
    MouseButton::WheelLeft,
    MouseButton::WheelRight,
    MouseButton::Back,
    MouseButton::Forward,
    MouseButton::Button10,
    MouseButton::Button11,
];

/// One key event, as Ghostty's embedding API reports it: what was
/// pressed and what text it typed; the core picks the bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_key_event {
    /// `sizeof(tt_key_event)` as the host knows it.
    pub struct_size: u32,
    /// The key: for a key of the main block, the code point it types on
    /// the US layout with no modifiers (`'a'`, `'1'`, `' '`); for any
    /// other key a `TT_KEY_*` code.
    pub key: u32,
    /// What the key types on the current layout with no modifiers (`с`
    /// for the `c` key on a Russian layout); 0 when it is `key` itself.
    pub unshifted: u32,
    /// `TT_KEY_PRESS`, `TT_KEY_REPEAT` or `TT_KEY_RELEASE`.
    pub action: u8,
    /// `TT_MOD_*` bits held, including this key if it is a modifier.
    pub mods: u8,
    /// `TT_MOD_*` bits the layout used to type `text` (Option typing `å`).
    pub consumed_mods: u8,
    /// 1 while an input method is composing: nothing is sent.
    pub composing: u8,
    /// UTF-8 the key typed, with Shift and consumed modifiers but not
    /// Control applied; may be empty.
    pub text: tt_str,
}

// SAFETY: repr(C), `struct_size` first, then integers and a `tt_str` of a
// raw pointer and an integer.
unsafe impl SizedStruct for tt_key_event {}

/// One mouse event, in cells and in pixels: the core picks which the
/// program gets.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_mouse_event {
    /// `sizeof(tt_mouse_event)` as the host knows it.
    pub struct_size: u32,
    /// `TT_MOUSE_PRESS`, `TT_MOUSE_RELEASE` or `TT_MOUSE_MOTION`.
    pub action: u8,
    /// The button pressed or released; for motion, the lowest one held or
    /// `TT_MOUSE_NONE`.
    pub button: u8,
    /// `TT_MOD_*` bits; Shift, Alt and Control are reported.
    pub mods: u8,
    /// Viewport column, from 0.
    pub col: u32,
    /// Viewport row, from 0.
    pub row: u32,
    /// Pixels from the left of the text area.
    pub x_px: u32,
    /// Pixels from the top of the text area.
    pub y_px: u32,
}

// SAFETY: repr(C), `struct_size` first, integers only.
unsafe impl SizedStruct for tt_mouse_event {}

/// The key behind a `tt_key_event.key`, or `None` for a code that names
/// none (a control character, a surrogate, an unused code).
fn key_of(code: u32) -> Option<Key> {
    if let Some(i) = code.checked_sub(TT_KEY_ESCAPE)
        && let Some(&key) = usize::try_from(i).ok().and_then(|i| NAMED.get(i))
    {
        return Some(key);
    }
    if let Some(n) = code.checked_sub(TT_KEY_F1).map(|i| i + 1)
        && n <= LAST_F_KEY
    {
        return u8::try_from(n).ok().map(Key::F);
    }
    if let Some(&key) = CODED.iter().find(|k| kitty_code(**k) == Some(code)) {
        return Some(key);
    }
    char::from_u32(code)
        .filter(|c| !c.is_control())
        .map(Key::Char)
}

/// The discriminant of the keys in [`CODED`].
fn kitty_code(key: Key) -> Option<u32> {
    match key {
        Key::Keypad(k) => Some(k as u32),
        Key::Media(k) => Some(k as u32),
        Key::Modifier(k) => Some(k as u32),
        Key::System(k) => Some(k as u32),
        _ => None,
    }
}

/// The key event behind `e`, borrowing its text; `None` when a field is
/// out of range or the text is not UTF-8.
///
/// # Safety
///
/// `e.text` is valid as for [`text`].
unsafe fn key_event<'a>(e: &tt_key_event) -> Option<KeyEvent<'a>> {
    let action = match e.action {
        TT_KEY_PRESS => KeyAction::Press,
        TT_KEY_REPEAT => KeyAction::Repeat,
        TT_KEY_RELEASE => KeyAction::Release,
        _ => return None,
    };
    let unshifted = match e.unshifted {
        0 => None,
        c => Some(char::from_u32(c)?),
    };
    Some(KeyEvent {
        key: key_of(e.key)?,
        action,
        mods: Modifiers::from_bits_retain(e.mods),
        consumed: Modifiers::from_bits_retain(e.consumed_mods),
        // SAFETY: the caller's contract.
        text: unsafe { text(e.text) }?,
        unshifted,
        composing: e.composing != 0,
    })
}

/// The mouse event behind `e`, or `None` when a field is out of range.
fn mouse_event(e: &tt_mouse_event) -> Option<MouseEvent> {
    let action = match e.action {
        TT_MOUSE_PRESS => MouseAction::Press,
        TT_MOUSE_RELEASE => MouseAction::Release,
        TT_MOUSE_MOTION => MouseAction::Motion,
        _ => return None,
    };
    let button = match e.button {
        TT_MOUSE_NONE => None,
        n => Some(*BUTTONS.get(usize::from(n) - 1)?),
    };
    // Only motion may come without a button.
    if button.is_none() && action != MouseAction::Motion {
        return None;
    }
    Some(MouseEvent {
        action,
        button,
        mods: Modifiers::from_bits_retain(e.mods),
        col: e.col,
        row: e.row,
        x_px: e.x_px,
        y_px: e.y_px,
    })
}

/// Encodes under the terminal lock, then hands the bytes to the child
/// whole: a report cut in two would reach it as a broken sequence. With
/// `follow`, input that sends bytes also brings the viewport back to the
/// screen, where what was typed shows.
fn send(t: &tt_term, follow: bool, encode: impl FnOnce(&Terminal, &mut Vec<u8>)) -> tt_status {
    let mut bytes = Vec::new();
    {
        let term = &mut t.core().lock().term;
        encode(term, &mut bytes);
        if follow && !bytes.is_empty() {
            term.scroll_display(isize::MIN);
        }
    }
    if bytes.is_empty() {
        return tt_status::TT_OK;
    }
    let Some(pty) = t.pty() else {
        return tt_status::TT_CLOSED;
    };
    match pty.write_input_whole(&bytes) {
        Ok(true) => tt_status::TT_OK,
        Ok(false) => tt_status::TT_FULL,
        Err(PtyError::Closed) => tt_status::TT_CLOSED,
        Err(_) => tt_status::TT_IO,
    }
}

/// Sends a key event. `TT_OK` also when the active modes send nothing
/// for it (a release in legacy mode, a composing key); `TT_CLOSED` when
/// there is no child; `TT_FULL` when the child is not reading and its
/// input queue lacks room, in which case nothing was sent.
///
/// # Safety
///
/// `term` is `NULL` or live; `event` is `NULL` or points to `struct_size`
/// readable bytes whose `text` is valid for its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_key(
    term: *const tt_term,
    event: *const tt_key_event,
) -> tt_status {
    let body = |t: &tt_term| {
        // SAFETY: the caller's contract.
        let Some(raw) = (unsafe { read_sized(event) }) else {
            return tt_status::TT_INVALID;
        };
        // SAFETY: the caller's contract.
        let Some(event) = (unsafe { key_event(&raw) }) else {
            return tt_status::TT_INVALID;
        };
        send(t, true, |term, out| term.encode_key(&event, out))
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// The `len` bytes at `bytes` as text, or `None` for `NULL` with a length
/// or bytes that are not UTF-8.
///
/// # Safety
///
/// `bytes` points to `len` readable bytes, or `len` is 0.
unsafe fn host_text<'a>(bytes: *const u8, len: usize) -> Option<&'a str> {
    // SAFETY: the caller's contract.
    unsafe { text(tt_str { ptr: bytes, len }) }
}

/// Sends text an input method committed outside a key event (dictation,
/// the character viewer). Control characters are dropped. Statuses as for
/// `tt_term_key`; `TT_INVALID` for text that is not UTF-8.
///
/// # Safety
///
/// `term` is `NULL` or live; `bytes` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_text(
    term: *const tt_term,
    bytes: *const u8,
    len: usize,
) -> tt_status {
    let body = |t: &tt_term| {
        // SAFETY: the caller's contract.
        let Some(text) = (unsafe { host_text(bytes, len) }) else {
            return tt_status::TT_INVALID;
        };
        send(t, true, |term, out| term.encode_text(text, out))
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Sends pasted text: bracketed, with any bracket marker inside removed,
/// while the program asks for it (mode 2004); otherwise line feeds become
/// carriage returns. A paste is sent whole or not at all, so one larger
/// than the child's input queue (256 KiB) answers `TT_FULL`. Statuses
/// otherwise as for `tt_term_text`.
///
/// # Safety
///
/// `term` is `NULL` or live; `bytes` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_paste(
    term: *const tt_term,
    bytes: *const u8,
    len: usize,
) -> tt_status {
    let body = |t: &tt_term| {
        // SAFETY: the caller's contract.
        let Some(text) = (unsafe { host_text(bytes, len) }) else {
            return tt_status::TT_INVALID;
        };
        send(t, true, |term, out| term.encode_paste(text, out))
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Tells the program the view gained (`focused` 1) or lost (0) focus,
/// while it asks for focus reports (mode 1004). Statuses as for
/// `tt_term_key`.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_focus(term: *const tt_term, focused: u8) -> tt_status {
    let body = |t: &tt_term| send(t, false, |term, out| term.encode_focus(focused != 0, out));
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Sends a mouse event if the program takes it. `*taken`, unless `NULL`,
/// becomes 1 when it does: always while it tracks the mouse, and for a
/// wheel step on the alternate screen, which becomes a cursor key (mode
/// 1007). At 0 the event is the host's: select, or scroll the history
/// with `tt_term_scroll_display`. Statuses as for `tt_term_key`.
///
/// # Safety
///
/// `term` is `NULL` or live; `event` is `NULL` or points to `struct_size`
/// readable bytes; `taken` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_mouse(
    term: *const tt_term,
    event: *const tt_mouse_event,
    taken: *mut u8,
) -> tt_status {
    let body = |t: &tt_term| {
        // SAFETY: the caller's contract.
        let raw = unsafe { read_sized(event) };
        let Some(event) = raw.as_ref().and_then(mouse_event) else {
            return tt_status::TT_INVALID;
        };
        let mut took = false;
        let status = send(t, false, |term, out| took = term.encode_mouse(&event, out));
        if !taken.is_null() {
            // SAFETY: non-null and writable by the caller's contract.
            unsafe { taken.write(u8::from(took)) };
        }
        status
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Moves the viewport `delta` rows back into history (negative: towards
/// the screen), clamped to what history holds. Typing, text and paste
/// bring it back to the screen by themselves.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_scroll_display(term: *const tt_term, delta: i32) -> tt_status {
    let body = |t: &tt_term| {
        let delta = isize::try_from(delta).unwrap_or(isize::MIN);
        t.core().lock().term.scroll_display(delta);
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;
    use crate::guard::zeroed;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    fn key(code: u32, text: &str) -> tt_key_event {
        tt_key_event {
            struct_size: u32::try_from(size_of::<tt_key_event>()).unwrap(),
            key: code,
            action: TT_KEY_PRESS,
            text: tt_str {
                ptr: text.as_ptr(),
                len: text.len(),
            },
            ..zeroed()
        }
    }

    fn click(action: u8, button: u8) -> tt_mouse_event {
        tt_mouse_event {
            struct_size: u32::try_from(size_of::<tt_mouse_event>()).unwrap(),
            action,
            button,
            col: 1,
            row: 2,
            ..zeroed()
        }
    }

    #[test]
    fn every_key_code_names_its_key() {
        assert_eq!(key_of(TT_KEY_ESCAPE), Some(Key::Escape));
        assert_eq!(key_of(TT_KEY_END), Some(Key::End));
        assert_eq!(key_of(TT_KEY_F1), Some(Key::F(1)));
        assert_eq!(key_of(TT_KEY_F1 + 34), Some(Key::F(35)));
        assert_eq!(
            key_of(TT_KEY_CAPS_LOCK),
            Some(Key::System(SystemKey::CapsLock))
        );
        assert_eq!(
            key_of(TT_KEY_KP_0 + 15),
            Some(Key::Keypad(KeypadKey::Enter))
        );
        assert_eq!(key_of(TT_KEY_MEDIA_PLAY), Some(Key::Media(MediaKey::Play)));
        let last = TT_KEY_LEFT_SHIFT + 13;
        assert_eq!(
            key_of(last),
            Some(Key::Modifier(ModifierKey::IsoLevel5Shift))
        );
        assert_eq!(key_of(u32::from('a')), Some(Key::Char('a')));
        for k in CODED {
            assert_eq!(key_of(kitty_code(k).unwrap()), Some(k));
        }
        assert_eq!(key_of(last + 1), char::from_u32(last + 1).map(Key::Char));
        assert_eq!(key_of(0x1b), None, "a control character is no key");
        assert_eq!(key_of(0xD800), None, "a surrogate is no key");
        assert_eq!(key_of(u32::MAX), None);
    }

    #[test]
    fn key_events_out_of_range_are_refused() {
        let ok = key(u32::from('a'), "a");
        // SAFETY: `ok.text` points to its bytes.
        let event = unsafe { key_event(&ok) }.unwrap();
        assert_eq!((event.key, event.text), (Key::Char('a'), "a"));
        let bad_action = tt_key_event { action: 0, ..ok };
        let bad_unshifted = tt_key_event {
            unshifted: 0xD800,
            ..ok
        };
        let bad_text = tt_key_event {
            text: tt_str {
                ptr: [0xFF_u8].as_ptr(),
                len: 1,
            },
            ..ok
        };
        for bad in [bad_action, bad_unshifted, bad_text] {
            // SAFETY: every text points to its bytes.
            assert!(unsafe { key_event(&bad) }.is_none(), "{bad:?}");
        }
    }

    #[test]
    fn mouse_events_need_a_known_action_and_button() {
        let left = mouse_event(&click(TT_MOUSE_PRESS, TT_MOUSE_LEFT)).unwrap();
        assert_eq!(left.button, Some(MouseButton::Left));
        assert_eq!((left.col, left.row), (1, 2));
        let wheel = mouse_event(&click(TT_MOUSE_PRESS, TT_MOUSE_WHEEL_RIGHT)).unwrap();
        assert_eq!(wheel.button, Some(MouseButton::WheelRight));
        assert!(mouse_event(&click(TT_MOUSE_MOTION, TT_MOUSE_NONE)).is_some());
        assert!(mouse_event(&click(TT_MOUSE_PRESS, TT_MOUSE_NONE)).is_none());
        assert!(mouse_event(&click(TT_MOUSE_PRESS, 12)).is_none());
        assert!(mouse_event(&click(0, TT_MOUSE_LEFT)).is_none());
    }

    #[test]
    fn input_without_a_child_is_closed_unless_nothing_is_sent() {
        let term = new_term(10, 3);
        let mut taken = 2;
        // SAFETY: live handle, whole structs, writable `taken`.
        unsafe {
            assert_eq!(
                tt_term_key(term, &key(u32::from('a'), "a")),
                tt_status::TT_CLOSED
            );
            let release = tt_key_event {
                action: TT_KEY_RELEASE,
                ..key(u32::from('a'), "")
            };
            assert_eq!(tt_term_key(term, &release), tt_status::TT_OK);
            assert_eq!(tt_term_text(term, b"x".as_ptr(), 1), tt_status::TT_CLOSED);
            assert_eq!(tt_term_text(term, b"\r".as_ptr(), 1), tt_status::TT_OK);
            assert_eq!(tt_term_paste(term, b"x".as_ptr(), 1), tt_status::TT_CLOSED);
            assert_eq!(tt_term_focus(term, 1), tt_status::TT_OK, "1004 is off");
            let wheel = click(TT_MOUSE_PRESS, TT_MOUSE_WHEEL_UP);
            assert_eq!(tt_term_mouse(term, &wheel, &mut taken), tt_status::TT_OK);
            assert_eq!(taken, 0);
            feed(term, b"\x1b[?1000h");
            assert_eq!(
                tt_term_mouse(term, &wheel, &mut taken),
                tt_status::TT_CLOSED
            );
            assert_eq!(taken, 1);
            assert_eq!(
                tt_term_mouse(term, &wheel, ptr::null_mut()),
                tt_status::TT_CLOSED
            );
            tt_term_free(term);
        }
    }

    #[test]
    fn bad_arguments_are_invalid_and_a_poisoned_terminal_says_so() {
        let term = new_term(10, 3);
        let tiny = tt_key_event {
            struct_size: 2,
            ..key(u32::from('a'), "a")
        };
        // SAFETY: live handle; NULL and short structs are allowed.
        unsafe {
            assert_eq!(tt_term_key(term, ptr::null()), tt_status::TT_INVALID);
            assert_eq!(tt_term_key(term, &tiny), tt_status::TT_INVALID);
            assert_eq!(tt_term_key(term, &key(0x1b, "")), tt_status::TT_INVALID);
            assert_eq!(tt_term_text(term, ptr::null(), 1), tt_status::TT_INVALID);
            assert_eq!(
                tt_term_paste(term, [0xFF].as_ptr(), 1),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_mouse(term, ptr::null(), ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_scroll_display(ptr::null(), 1),
                tt_status::TT_INVALID
            );
            assert_eq!(tt_term_focus(ptr::null(), 1), tt_status::TT_INVALID);
            let _ = with_term(term, |_| panic!("core bug"));
            assert_eq!(tt_term_text(term, ptr::null(), 0), tt_status::TT_POISONED);
            assert_eq!(tt_term_scroll_display(term, 1), tt_status::TT_POISONED);
            tt_term_free(term);
        }
    }

    #[test]
    fn the_viewport_scrolls_back_and_typing_brings_it_home() {
        let term = new_term(10, 2);
        feed(term, b"1\r\n2\r\n3\r\n4");
        let offset = || {
            // SAFETY: live handle.
            unsafe { &*term }.core().lock().term.grid().display_offset()
        };
        // SAFETY: live handle and whole structs.
        unsafe {
            assert_eq!(tt_term_scroll_display(term, i32::MAX), tt_status::TT_OK);
            assert_eq!(offset(), 2, "clamped to the history");
            assert_eq!(tt_term_scroll_display(term, -1), tt_status::TT_OK);
            assert_eq!(offset(), 1);
            // The focus report goes nowhere, so it leaves the view alone.
            feed(term, b"\x1b[?1004h");
            let _ = tt_term_focus(term, 1);
            assert_eq!(offset(), 1);
            let _ = tt_term_key(term, &key(u32::from('a'), "a"));
            assert_eq!(offset(), 0);
            tt_term_free(term);
        }
    }
}
