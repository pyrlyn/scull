//! Setting and reading modes by number: SM, RM, DECSET, DECRST and the
//! state DECRQM reports. Separate from `modes.rs`, which is the plain value
//! other crates read, because switching some modes moves the cursor or
//! changes screens.
//!
//! Side effects follow xterm's ctlseqs: DECOM homes the cursor both ways;
//! resetting DECAWM drops a pending wrap (DEC STD 070 "last column flag");
//! resetting DECLRMM removes the left and right margins. Resetting any
//! mouse tracking mode turns tracking off and resetting any encoding goes
//! back to the default one, whichever was set: xterm and Ghostty do the
//! same, so a program that leaves with the wrong reset still frees the
//! mouse.

use scull_input::{MouseEncoding, MouseTracking};

use crate::modes::{AnsiMode, DecMode};
use crate::state::State;

impl State {
    /// SM and RM. Unknown modes are ignored.
    pub(crate) fn set_ansi_mode(&mut self, code: u16, on: bool) {
        match AnsiMode::from_code(code) {
            Some(AnsiMode::Insert) => self.modes.insert = on,
            Some(AnsiMode::Newline) => self.modes.newline = on,
            None => {}
        }
    }

    /// DECSET and DECRST. Unknown modes are ignored.
    pub(crate) fn set_dec_mode(&mut self, code: u16, on: bool) {
        let Some(mode) = DecMode::from_code(code) else {
            return;
        };
        match mode {
            DecMode::CursorKeys => self.modes.cursor_keys = on,
            DecMode::Origin => {
                self.modes.origin = on;
                self.goto_origin(0, 0);
            }
            DecMode::Autowrap => {
                self.modes.autowrap = on;
                if !on {
                    self.cursor.pending_wrap = false;
                }
            }
            DecMode::CursorVisible => self.modes.cursor_visible = on,
            DecMode::AltScreen => {
                if on {
                    self.enter_alt_screen(false);
                } else {
                    self.leave_alt_screen(false);
                }
            }
            DecMode::LeftRightMargins => {
                self.modes.left_right_margins = on;
                if !on {
                    self.margins.left = 0;
                    self.margins.right = self.last_col();
                }
            }
            DecMode::X10Mouse | DecMode::NormalMouse | DecMode::ButtonMouse | DecMode::AnyMouse => {
                let tracking = mode.mouse_tracking().filter(|_| on);
                self.modes.mouse.tracking = tracking.unwrap_or(MouseTracking::Off);
            }
            DecMode::Utf8Mouse
            | DecMode::SgrMouse
            | DecMode::UrxvtMouse
            | DecMode::SgrPixelMouse => {
                let encoding = mode.mouse_encoding().filter(|_| on);
                self.modes.mouse.encoding = encoding.unwrap_or(MouseEncoding::Default);
            }
            DecMode::AlternateScroll => self.modes.alternate_scroll = on,
            DecMode::FocusEvents => self.modes.focus_events = on,
            DecMode::AltScreenClear => {
                if on {
                    self.enter_alt_screen(false);
                } else {
                    self.leave_alt_screen(true);
                }
            }
            DecMode::SaveCursor => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            DecMode::AltScreenSaveCursor => {
                if on {
                    if !self.alt_active {
                        self.save_cursor();
                    }
                    self.enter_alt_screen(true);
                } else if self.alt_active {
                    self.leave_alt_screen(false);
                    self.restore_cursor();
                }
            }
            DecMode::BracketedPaste => self.modes.bracketed_paste = on,
            DecMode::SynchronizedOutput => self.modes.synchronized_output = on,
            DecMode::GraphemeClusters => self.modes.grapheme_clusters = on,
        }
    }

    /// Whether a DEC private mode is set, for DECRQM. `?1048` has no state
    /// of its own and reads as reset, as in xterm.
    pub(crate) fn dec_mode(&self, mode: DecMode) -> bool {
        let m = &self.modes;
        match mode {
            DecMode::CursorKeys => m.cursor_keys,
            DecMode::Origin => m.origin,
            DecMode::Autowrap => m.autowrap,
            DecMode::CursorVisible => m.cursor_visible,
            DecMode::AltScreen | DecMode::AltScreenClear | DecMode::AltScreenSaveCursor => {
                self.alt_active
            }
            DecMode::LeftRightMargins => m.left_right_margins,
            DecMode::X10Mouse | DecMode::NormalMouse | DecMode::ButtonMouse | DecMode::AnyMouse => {
                mode.mouse_tracking() == Some(m.mouse.tracking)
            }
            DecMode::Utf8Mouse
            | DecMode::SgrMouse
            | DecMode::UrxvtMouse
            | DecMode::SgrPixelMouse => mode.mouse_encoding() == Some(m.mouse.encoding),
            DecMode::AlternateScroll => m.alternate_scroll,
            DecMode::FocusEvents => m.focus_events,
            DecMode::SaveCursor => false,
            DecMode::BracketedPaste => m.bracketed_paste,
            DecMode::SynchronizedOutput => m.synchronized_output,
            DecMode::GraphemeClusters => m.grapheme_clusters,
        }
    }
}
