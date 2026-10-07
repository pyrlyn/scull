//! The two screens and what survives switching between them: the alternate
//! screen, the saved cursor (DECSC, DECRC), and the resets (RIS, DECSTR).
//! Separate because these replace whole parts of the state at once rather
//! than editing cells.
//!
//! Follows xterm's ctlseqs and the esctest2 cases in `tests/README.md`:
//! each screen has its own saved cursor; the alternate screen has no
//! scrollback; DECSC saves the position (with the pending wrap), the pen,
//! the character sets and origin mode, but not DECAWM or IRM; DECRC with
//! nothing saved homes the cursor and resets those. DECSTR keeps DECAWM
//! and the cursor position, as xterm does; RIS keeps the scrollback.

use scull_grid::{Cell, Grid, Style};

use crate::charset::Charsets;
use crate::modes::Modes;
use crate::state::{Cursor, Margins, State};
use crate::tabs::TabStops;

/// What DECSC remembers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SavedCursor {
    cursor: Cursor,
    style: Style,
    origin: bool,
    charsets: Charsets,
}

fn clear_screen(grid: &mut Grid) {
    for r in 0..grid.screen_rows() {
        if let Some(row) = grid.screen_row_mut(r) {
            row.clear(Cell::EMPTY);
        }
    }
}

impl State {
    fn saved_slot(&mut self) -> &mut Option<SavedCursor> {
        &mut self.saved[usize::from(self.alt_active)]
    }

    /// DECSC, SCOSC and `?1048` set.
    pub(crate) fn save_cursor(&mut self) {
        let saved = SavedCursor {
            cursor: self.cursor,
            style: *self.pen.style(),
            origin: self.modes.origin,
            charsets: self.charsets,
        };
        *self.saved_slot() = Some(saved);
    }

    /// DECRC, SCORC and `?1048` reset.
    pub(crate) fn restore_cursor(&mut self) {
        let saved = self.saved_slot().unwrap_or_default();
        self.goto(saved.cursor.row, saved.cursor.col);
        self.cursor.pending_wrap = saved.cursor.pending_wrap && self.modes.autowrap;
        self.pen.set(saved.style);
        self.modes.origin = saved.origin;
        self.charsets = saved.charsets;
    }

    /// Swaps the active and inactive grids. Style ids belong to one grid, so
    /// the pen must intern again on the other.
    fn swap_screens(&mut self) {
        std::mem::swap(&mut self.grid, &mut self.alt);
        self.alt_active = !self.alt_active;
        self.pen.forget();
    }

    /// Enters the alternate screen; `clear` blanks it first (`?1049`).
    pub(crate) fn enter_alt_screen(&mut self, clear: bool) {
        if !self.alt_active {
            self.swap_screens();
        }
        if clear {
            clear_screen(&mut self.grid);
        }
    }

    /// Leaves the alternate screen; `clear` blanks it on the way (`?1047`).
    pub(crate) fn leave_alt_screen(&mut self, clear: bool) {
        if self.alt_active {
            if clear {
                clear_screen(&mut self.grid);
            }
            self.swap_screens();
        }
    }

    /// RIS: back to the power-on state, keeping the scrollback and any
    /// replies not yet drained.
    pub(crate) fn full_reset(&mut self) {
        self.leave_alt_screen(true);
        clear_screen(&mut self.grid);
        let (cols, rows) = (self.cols(), self.rows());
        self.cursor = Cursor::default();
        self.pen.set(Style::default());
        self.modes = Modes::default();
        self.margins = Margins::full(cols, rows);
        self.tabs = TabStops::new(cols);
        self.charsets = Charsets::default();
        self.saved = [None, None];
        self.last_char = None;
    }

    /// DECSTR: the soft reset of DEC STD 070 as xterm applies it. Modes it
    /// does not list (DECAWM, LNM, paste, focus, 2026, 2027) are kept.
    pub(crate) fn soft_reset(&mut self) {
        let power_on = Modes::default();
        self.modes = Modes {
            insert: power_on.insert,
            origin: power_on.origin,
            cursor_keys: power_on.cursor_keys,
            keypad_app: power_on.keypad_app,
            cursor_visible: power_on.cursor_visible,
            left_right_margins: power_on.left_right_margins,
            ..self.modes
        };
        self.margins = Margins::full(self.cols(), self.rows());
        self.pen.set(Style::default());
        self.charsets = Charsets::default();
        *self.saved_slot() = None;
    }
}
