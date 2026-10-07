//! Editing the screen: erase (ED, EL, ECH), insert and delete (ICH, DCH,
//! IL, DL), region scrolling (SU, SD and the scrolls of IND and RI), and
//! DECALN. Separate from motion because these change cells.
//!
//! Erased cells keep the pen's background (xterm's background colour
//! erase). Every edit clears the pending wrap, as xterm does. Rules for the
//! margins follow xterm's ctlseqs and DEC STD 070: ECH, ED and EL ignore
//! them; ICH, DCH, IL and DL do nothing when the cursor is outside them.

use scull_grid::{Cell, StyleId};

use crate::state::{Margins, State};

/// DECALN fills the screen with this letter.
const ALIGNMENT_CHAR: char = 'E';

/// Which part of the line or display an erase covers (`Ps` of ED and EL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Erase {
    /// From the cursor to the end.
    ToEnd = 0,
    /// From the start to the cursor, inclusive.
    ToStart = 1,
    /// All of it.
    All = 2,
    /// ED 3 (xterm): the scrollback, leaving the screen alone.
    Scrollback = 3,
}

impl Erase {
    pub(crate) fn from_param(ps: u16) -> Option<Self> {
        [Self::ToEnd, Self::ToStart, Self::All, Self::Scrollback]
            .into_iter()
            .find(|e| *e as u16 == ps)
    }
}

impl State {
    /// EL.
    pub(crate) fn erase_line(&mut self, which: Erase) {
        self.cursor.pending_wrap = false;
        let col = self.cursor.col;
        let cols = match which {
            Erase::ToEnd => col..self.cols(),
            Erase::ToStart => 0..col.saturating_add(1),
            Erase::All => 0..self.cols(),
            Erase::Scrollback => return,
        };
        let blank = self.blank();
        if let Some(row) = self.cursor_row() {
            row.fill_range(cols, blank);
        }
    }

    /// ED.
    pub(crate) fn erase_display(&mut self, which: Erase) {
        let rows = match which {
            Erase::ToEnd => self.cursor.row.saturating_add(1)..self.rows(),
            Erase::ToStart => 0..self.cursor.row,
            Erase::All => 0..self.rows(),
            Erase::Scrollback => {
                self.grid.clear_history();
                return;
            }
        };
        if which != Erase::All {
            self.erase_line(which);
        }
        self.cursor.pending_wrap = false;
        let blank = self.blank();
        for r in rows {
            if let Some(row) = self.grid.screen_row_mut(r) {
                row.clear(blank);
            }
        }
    }

    /// ECH: blanks `n` cells from the cursor without moving anything.
    pub(crate) fn erase_chars(&mut self, n: u16) {
        self.cursor.pending_wrap = false;
        let col = self.cursor.col;
        let blank = self.blank();
        if let Some(row) = self.cursor_row() {
            row.fill_range(col..col.saturating_add(n), blank);
        }
    }

    /// ICH (and IRM before a character): opens `n` blank cells at the
    /// cursor, pushing the rest of the line towards the right margin.
    pub(crate) fn insert_chars(&mut self, n: u16) {
        self.shift_chars(n, true);
    }

    /// DCH: removes `n` cells at the cursor, pulling the rest of the line in
    /// from the right margin.
    pub(crate) fn delete_chars(&mut self, n: u16) {
        self.shift_chars(n, false);
    }

    fn shift_chars(&mut self, n: u16, insert: bool) {
        self.cursor.pending_wrap = false;
        if !self.in_lr_margins() {
            return;
        }
        let cols = self.cursor.col..self.margins.right.saturating_add(1);
        let blank = self.blank();
        if let Some(row) = self.cursor_row() {
            if insert {
                row.insert_cells(cols, n, blank);
            } else {
                row.delete_cells(cols, n, blank);
            }
        }
    }

    /// IL: `n` blank lines at the cursor row, pushing the region down.
    pub(crate) fn insert_lines(&mut self, n: u16) {
        self.shift_lines(n, true);
    }

    /// DL: removes `n` lines at the cursor row, pulling the region up.
    pub(crate) fn delete_lines(&mut self, n: u16) {
        self.shift_lines(n, false);
    }

    /// IL and DL act on the rows from the cursor to the bottom margin and
    /// leave the cursor at the left margin (DEC STD 070).
    fn shift_lines(&mut self, n: u16, insert: bool) {
        self.cursor.pending_wrap = false;
        if !self.in_tb_margins() || !self.in_lr_margins() {
            return;
        }
        let region = Margins {
            top: self.cursor.row,
            ..self.margins
        };
        if insert {
            self.scroll_region_down(region, n);
        } else {
            self.scroll_region_up(region, n);
        }
        self.cursor.col = self.margins.left;
    }

    /// SU and the scroll of IND: the scroll region moves up `n` rows.
    pub(crate) fn scroll_up(&mut self, n: u16) {
        self.scroll_region_up(self.margins, n);
    }

    /// SD and the scroll of RI: the scroll region moves down `n` rows.
    pub(crate) fn scroll_down(&mut self, n: u16) {
        self.scroll_region_down(self.margins, n);
    }

    fn scroll_region_up(&mut self, region: Margins, n: u16) {
        let blank = self.blank();
        let rows = region.top..region.bottom.saturating_add(1);
        self.grid.scroll_region_up(rows, n, blank);
    }

    fn scroll_region_down(&mut self, region: Margins, n: u16) {
        let blank = self.blank();
        let rows = region.top..region.bottom.saturating_add(1);
        self.grid.scroll_region_down(rows, n, blank);
    }

    /// DECALN: the screen filled with `E` in the default style, margins
    /// reset, cursor home (VT100 screen alignment test).
    pub(crate) fn alignment_test(&mut self) {
        self.margins = Margins::full(self.cols(), self.rows());
        for r in 0..self.rows() {
            if let Some(row) = self.grid.screen_row_mut(r) {
                row.clear(Cell::char(ALIGNMENT_CHAR, StyleId::DEFAULT));
            }
        }
        self.goto(0, 0);
    }
}
