//! Input from the UI turned into bytes for the program against the modes
//! the program set (keys, text, paste, focus, mouse), and the kitty
//! keyboard stack the program drives. Separate from `set_mode.rs`, which
//! writes the modes: this is where they are read on the way back to the
//! program. The encoders are scull-input's; only the choice of modes and
//! the policy around them (alternate scroll, focus reports off) live here.

use scull_input::{
    Key, KeyEvent, KeyModes, KittyFlagStack, MouseAction, MouseButton, MouseEvent, MouseModes,
    MouseTracking, Screen, encode_focus, encode_key, encode_mouse, encode_paste,
};

use crate::state::State;
use crate::terminal::Terminal;

impl State {
    fn screen(&self) -> Screen {
        if self.alt_active {
            Screen::Alternate
        } else {
            Screen::Main
        }
    }

    /// The kitty flag stack of the screen shown: each screen has its own,
    /// so an editor on the alternate screen cannot disturb the shell's.
    pub(crate) fn kitty_stack(&mut self) -> &mut KittyFlagStack {
        let screen = self.screen();
        self.modes.kitty.stack_mut(screen)
    }

    /// `CSI ? u`: reports the active kitty flags.
    pub(crate) fn kitty_query(&mut self) {
        let mut reply = Vec::new();
        self.modes
            .kitty
            .stack(self.screen())
            .write_query_reply(&mut reply);
        self.replies.push(&reply);
    }
}

impl Terminal {
    /// The modes key encoding reads now. `modifyOtherKeys` and
    /// win32-input-mode are not kept yet, so they read as off.
    pub fn key_modes(&self) -> KeyModes {
        let s = &self.state;
        KeyModes {
            application_cursor: s.modes.cursor_keys,
            application_keypad: s.modes.keypad_app,
            kitty: s.modes.kitty.stack(s.screen()).current(),
            ..KeyModes::default()
        }
    }

    /// The mouse modes the program set.
    pub fn mouse_modes(&self) -> MouseModes {
        self.state.modes.mouse
    }

    /// Appends what the program gets for a key event: nothing for a
    /// release in legacy mode or a key being composed.
    pub fn encode_key(&self, event: &KeyEvent<'_>, out: &mut Vec<u8>) {
        encode_key(event, &self.key_modes(), out);
    }

    /// Appends text an input method committed, without control
    /// characters: text is not a key, so it must not type an escape
    /// sequence or a line break the user never pressed.
    pub fn encode_text(&self, text: &str, out: &mut Vec<u8>) {
        let mut buf = [0_u8; char::MAX_LEN_UTF8];
        for c in text.chars().filter(|c| !c.is_control()) {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }

    /// Appends pasted text, bracketed while the program asked for it
    /// (`?2004`).
    pub fn encode_paste(&self, text: &str, out: &mut Vec<u8>) {
        encode_paste(text, self.state.modes.bracketed_paste, out);
    }

    /// Appends a focus report while the program asked for them (`?1004`).
    pub fn encode_focus(&self, focused: bool, out: &mut Vec<u8>) {
        if self.state.modes.focus_events {
            encode_focus(focused, out);
        }
    }

    /// Appends what the program gets for a mouse event and says whether
    /// the program took it: always while it tracks the mouse, and for a
    /// wheel step that alternate scroll (`?1007`) turns into a cursor key
    /// on the alternate screen. False leaves the event to the UI, which
    /// selects or scrolls the history.
    pub fn encode_mouse(&self, event: &MouseEvent, out: &mut Vec<u8>) -> bool {
        let modes = &self.state.modes;
        if modes.mouse.tracking != MouseTracking::Off {
            encode_mouse(event, &modes.mouse, out);
            return true;
        }
        if !(self.state.alt_active && modes.alternate_scroll) {
            return false;
        }
        let key = match (event.action, event.button) {
            (MouseAction::Press, Some(MouseButton::WheelUp)) => Key::Up,
            (MouseAction::Press, Some(MouseButton::WheelDown)) => Key::Down,
            _ => return false,
        };
        self.encode_key(&KeyEvent::new(key), out);
        true
    }
}

#[cfg(test)]
mod tests {
    use scull_input::{KittyFlags, Modifiers, MouseEncoding};

    use super::*;

    fn term() -> Terminal {
        Terminal::new(10, 4, 10).unwrap()
    }

    fn key(t: &Terminal, key: Key) -> Vec<u8> {
        let mut out = Vec::new();
        t.encode_key(&KeyEvent::new(key), &mut out);
        out
    }

    fn mouse(t: &Terminal, action: MouseAction, button: MouseButton) -> (bool, Vec<u8>) {
        let event = MouseEvent {
            action,
            button: Some(button),
            mods: Modifiers::empty(),
            col: 2,
            row: 1,
            x_px: 20,
            y_px: 17,
        };
        let mut out = Vec::new();
        let taken = t.encode_mouse(&event, &mut out);
        (taken, out)
    }

    fn replies(t: &mut Terminal, bytes: &[u8]) -> Vec<u8> {
        t.feed(bytes);
        t.take_replies()
    }

    #[test]
    fn cursor_keys_follow_decckm() {
        let mut t = term();
        assert_eq!(key(&t, Key::Up), b"\x1b[A");
        t.feed(b"\x1b[?1h");
        assert_eq!(key(&t, Key::Up), b"\x1bOA");
    }

    #[test]
    fn the_keypad_follows_deckpam() {
        let mut t = term();
        t.feed(b"\x1b=");
        assert!(t.key_modes().application_keypad);
        t.feed(b"\x1b>");
        assert!(!t.key_modes().application_keypad);
    }

    #[test]
    fn kitty_flags_are_pushed_set_queried_and_popped() {
        let mut t = term();
        assert_eq!(replies(&mut t, b"\x1b[?u"), b"\x1b[?0u");
        t.feed(b"\x1b[>1u");
        assert_eq!(t.key_modes().kitty, KittyFlags::DISAMBIGUATE);
        assert_eq!(key(&t, Key::Escape), b"\x1b[27u");
        t.feed(b"\x1b[=8;2u");
        assert_eq!(replies(&mut t, b"\x1b[?u"), b"\x1b[?9u");
        t.feed(b"\x1b[=1;3u");
        assert_eq!(t.key_modes().kitty, KittyFlags::REPORT_ALL_KEYS);
        t.feed(b"\x1b[<u");
        assert!(t.key_modes().kitty.is_legacy());
        assert_eq!(key(&t, Key::Escape), b"\x1b");
    }

    #[test]
    fn each_screen_has_its_own_kitty_stack_and_a_full_reset_clears_both() {
        let mut t = term();
        t.feed(b"\x1b[>1u\x1b[?1049h");
        assert!(
            t.key_modes().kitty.is_legacy(),
            "the alternate screen starts empty"
        );
        t.feed(b"\x1b[>31u");
        assert_eq!(t.key_modes().kitty, KittyFlags::all());
        t.feed(b"\x1b[?1049l");
        assert_eq!(t.key_modes().kitty, KittyFlags::DISAMBIGUATE);
        t.feed(b"\x1bc");
        assert!(t.key_modes().kitty.is_legacy());
    }

    #[test]
    fn a_flood_of_pushes_stays_within_the_stack_depth() {
        let mut t = term();
        t.feed(&b"\x1b[>1u".repeat(1000));
        assert_eq!(
            t.state.modes.kitty.stack(Screen::Main).depth(),
            scull_input::KITTY_FLAG_STACK_DEPTH
        );
    }

    #[test]
    fn mouse_tracking_and_encoding_modes_are_kept_and_reported() {
        let mut t = term();
        for (set, tracking) in [
            (&b"\x1b[?9h"[..], MouseTracking::X10),
            (b"\x1b[?1000h", MouseTracking::Normal),
            (b"\x1b[?1002h", MouseTracking::ButtonEvent),
            (b"\x1b[?1003h", MouseTracking::AnyEvent),
        ] {
            t.feed(set);
            assert_eq!(t.mouse_modes().tracking, tracking);
        }
        assert_eq!(replies(&mut t, b"\x1b[?1003$p"), b"\x1b[?1003;1$y");
        assert_eq!(replies(&mut t, b"\x1b[?1000$p"), b"\x1b[?1000;2$y");
        for (set, encoding) in [
            (&b"\x1b[?1005h"[..], MouseEncoding::Utf8),
            (b"\x1b[?1015h", MouseEncoding::Urxvt),
            (b"\x1b[?1016h", MouseEncoding::SgrPixels),
            (b"\x1b[?1006h", MouseEncoding::Sgr),
        ] {
            t.feed(set);
            assert_eq!(t.mouse_modes().encoding, encoding);
        }
        assert_eq!(replies(&mut t, b"\x1b[?1006$p"), b"\x1b[?1006;1$y");
        assert_eq!(replies(&mut t, b"\x1b[?1016$p"), b"\x1b[?1016;2$y");
        // Any reset frees the mouse, as in xterm.
        t.feed(b"\x1b[?1000l\x1b[?1015l");
        assert_eq!(t.mouse_modes(), MouseModes::default());
    }

    #[test]
    fn a_tracked_mouse_is_reported_in_the_chosen_encoding() {
        let mut t = term();
        assert_eq!(
            mouse(&t, MouseAction::Press, MouseButton::Left),
            (false, Vec::new())
        );
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            mouse(&t, MouseAction::Press, MouseButton::Left),
            (true, b"\x1b[<0;3;2M".to_vec())
        );
        t.feed(b"\x1b[?1016h");
        assert_eq!(
            mouse(&t, MouseAction::Release, MouseButton::Left),
            (true, b"\x1b[<0;21;18m".to_vec())
        );
    }

    #[test]
    fn the_wheel_sends_cursor_keys_on_the_alternate_screen_until_1007_is_reset() {
        let mut t = term();
        assert_eq!(replies(&mut t, b"\x1b[?1007$p"), b"\x1b[?1007;1$y");
        let wheel = |t: &Terminal| mouse(t, MouseAction::Press, MouseButton::WheelUp);
        assert_eq!(
            wheel(&t),
            (false, Vec::new()),
            "the main screen scrolls history"
        );
        t.feed(b"\x1b[?1049h");
        assert_eq!(wheel(&t), (true, b"\x1b[A".to_vec()));
        t.feed(b"\x1b[?1h");
        let down = mouse(&t, MouseAction::Press, MouseButton::WheelDown);
        assert_eq!(down, (true, b"\x1bOB".to_vec()));
        assert_eq!(
            mouse(&t, MouseAction::Press, MouseButton::Left),
            (false, Vec::new())
        );
        t.feed(b"\x1b[?1007l");
        assert_eq!(wheel(&t), (false, Vec::new()));
    }

    #[test]
    fn paste_is_bracketed_only_in_2004() {
        let mut t = term();
        let mut out = Vec::new();
        t.encode_paste("a\nb", &mut out);
        assert_eq!(out, b"a\rb");
        t.feed(b"\x1b[?2004h");
        out.clear();
        t.encode_paste("x\x1b[201~y", &mut out);
        assert_eq!(out, b"\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn focus_is_reported_only_in_1004() {
        let mut t = term();
        let mut out = Vec::new();
        t.encode_focus(true, &mut out);
        assert!(out.is_empty());
        t.feed(b"\x1b[?1004h");
        t.encode_focus(true, &mut out);
        t.encode_focus(false, &mut out);
        assert_eq!(out, b"\x1b[I\x1b[O");
    }

    #[test]
    fn committed_text_loses_its_control_characters() {
        let t = term();
        let mut out = Vec::new();
        t.encode_text("é\x1b[31m\r\u{9b}ü", &mut out);
        assert_eq!(out, "é[31mü".as_bytes());
    }
}
