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
                let history = u64::try_from(self.grid.history_len()).unwrap_or(u64::MAX);
                self.images.evicted = self.images.evicted.saturating_add(history);
                self.grid.clear_history();
                return;
            }
        };
        if which == Erase::All {
            self.erase_screen_images();
        } else {
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
        if self.full_width(region) {
            let rows = region.top..region.bottom.saturating_add(1);
            let whole = region.top == 0 && region.bottom >= self.last_row();
            let history = self.grid.history_len();
            self.grid.scroll_region_up(rows, n, blank);
            // Only a scroll of the whole screen feeds (and evicts) history.
            if whole {
                self.count_evicted(n, history);
            }
        } else {
            self.scroll_columns(region, n, true, blank);
        }
    }

    fn scroll_region_down(&mut self, region: Margins, n: u16) {
        let blank = self.blank();
        if self.full_width(region) {
            let rows = region.top..region.bottom.saturating_add(1);
            self.grid.scroll_region_down(rows, n, blank);
        } else {
            self.scroll_columns(region, n, false, blank);
        }
    }

    fn full_width(&self, region: Margins) -> bool {
        region.left == 0 && region.right >= self.last_col()
    }

    /// Scrolling inside left and right margins (DECLRMM): only the cells
    /// between them move, row by row, and nothing reaches the scrollback,
    /// since no whole line leaves the screen (DEC STD 070, xterm).
    fn scroll_columns(&mut self, region: Margins, n: u16, up: bool, blank: Cell) {
        let height = region.bottom.saturating_sub(region.top).saturating_add(1);
        let n = n.min(height);
        let cols = region.left..region.right.saturating_add(1);
        let mut cells = Vec::with_capacity(usize::from(cols.end - cols.start));
        for i in 0..height - n {
            let (dst, src) = if up {
                (region.top + i, region.top + i + n)
            } else {
                (region.bottom - i, region.bottom - i - n)
            };
            cells.clear();
            if let Some(row) = self.grid.screen_row(src) {
                cells.extend(cols.clone().filter_map(|c| row.cell(c)));
            }
            if let Some(row) = self.grid.screen_row_mut(dst) {
                row.write_cells(region.left, &cells);
            }
        }
        let cleared = if up {
            region.bottom + 1 - n..region.bottom + 1
        } else {
            region.top..region.top + n
        };
        for r in cleared {
            if let Some(row) = self.grid.screen_row_mut(r) {
                row.fill_range(cols.clone(), blank);
            }
        }
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
