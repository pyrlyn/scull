//! Cursor movement: absolute and relative moves, carriage return, index and
//! reverse index, tabs. Separate from editing because none of these change a
//! cell; IND and RI only scroll.
//!
//! Margin rules follow xterm and DEC STD 070 (and the esctest2 cases listed
//! in `tests/README.md`): a relative move stops at a margin only when it
//! starts inside it, absolute moves honour origin mode (DECOM), and every
//! move clears the pending wrap.

use crate::state::State;

impl State {
    /// Puts the cursor at an absolute screen position, clamped to the screen.
    pub(crate) fn goto(&mut self, row: u16, col: u16) {
        self.cursor.row = row.min(self.last_row());
        self.cursor.col = col.min(self.last_col());
        self.cursor.pending_wrap = false;
    }

    /// CUP and HVP: `row` and `col` count from 0 at the origin, which is the
    /// top-left margin corner in origin mode, and are clamped into it.
    pub(crate) fn goto_origin(&mut self, row: u16, col: u16) {
        let m = self.margins;
        if self.modes.origin {
            self.goto(
                m.top.saturating_add(row).min(m.bottom),
                m.left.saturating_add(col).min(m.right),
            );
        } else {
            self.goto(row, col);
        }
    }

    /// CHA and HPA: column `col` of the current row, origin-relative.
    pub(crate) fn goto_col(&mut self, col: u16) {
        let row = self.cursor.row;
        let m = self.margins;
        if self.modes.origin {
            self.goto(row, m.left.saturating_add(col).min(m.right));
        } else {
            self.goto(row, col);
        }
    }

    /// VPA: row `row` in the current column, origin-relative.
    pub(crate) fn goto_row(&mut self, row: u16) {
        let col = self.cursor.col;
        let m = self.margins;
        if self.modes.origin {
            self.goto(m.top.saturating_add(row).min(m.bottom), col);
        } else {
            self.goto(row, col);
        }
    }

    /// CUU: up `n`, stopping at the top margin when starting below it.
    pub(crate) fn up(&mut self, n: u16) {
        let floor = if self.cursor.row >= self.margins.top {
            self.margins.top
        } else {
            0
        };
        let row = self.cursor.row.saturating_sub(n).max(floor);
        self.goto(row, self.cursor.col);
    }

    /// CUD: down `n`, stopping at the bottom margin when starting above it.
    pub(crate) fn down(&mut self, n: u16) {
        let ceiling = if self.cursor.row <= self.margins.bottom {
            self.margins.bottom
        } else {
            self.last_row()
        };
        let row = self.cursor.row.saturating_add(n).min(ceiling);
        self.goto(row, self.cursor.col);
    }

    /// CUF: right `n`, stopping at the right margin when starting left of it.
    pub(crate) fn forward(&mut self, n: u16) {
        let ceiling = self.line_end();
        let col = self.cursor.col.saturating_add(n).min(ceiling);
        self.goto(self.cursor.row, col);
    }

    /// CUB and BS: left `n`, stopping at the left margin when starting right
    /// of it. From a pending wrap the cursor leaves the last column, as
    /// xterm does without reverse wraparound.
    pub(crate) fn back(&mut self, n: u16) {
        let floor = if self.cursor.col >= self.margins.left {
            self.margins.left
        } else {
            0
        };
        let col = self.cursor.col.saturating_sub(n).max(floor);
        self.goto(self.cursor.row, col);
    }

    /// HPR and VPR: relative moves bounded by the screen, not the margins.
    pub(crate) fn relative(&mut self, rows: u16, cols: u16) {
        let row = self.cursor.row.saturating_add(rows);
        let col = self.cursor.col.saturating_add(cols);
        self.goto(row, col);
    }

    /// CR: to the left margin, or column 0 when already left of it.
    pub(crate) fn carriage_return(&mut self) {
        let col = if self.cursor.col >= self.margins.left {
            self.margins.left
        } else {
            0
        };
        self.goto(self.cursor.row, col);
    }

    /// IND and LF: down one row; at the bottom margin the region scrolls up
    /// instead, if the cursor is inside the left and right margins.
    pub(crate) fn index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.margins.bottom {
            if self.in_lr_margins() {
                self.scroll_up(1);
            }
        } else if self.cursor.row < self.last_row() {
            self.cursor.row += 1;
        }
    }

    /// RI: up one row; at the top margin the region scrolls down instead.
    pub(crate) fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.margins.top {
            if self.in_lr_margins() {
                self.scroll_down(1);
            }
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    /// LF, VT and FF: an index, plus a carriage return under LNM.
    pub(crate) fn linefeed(&mut self) {
        self.index();
        if self.modes.newline {
            self.carriage_return();
        }
    }

    /// HT and CHT: forward `n` tab stops, never past the right margin and
    /// never wrapping. The pending wrap is kept: it can only be set at the
    /// line end, where a tab does not move.
    pub(crate) fn tab(&mut self, n: u16) {
        let end = self.line_end();
        self.cursor.col = self.tabs.next(self.cursor.col, n, end);
    }

    /// CBT: back `n` tab stops, down to column 0 whatever the margins.
    pub(crate) fn back_tab(&mut self, n: u16) {
        let col = self.tabs.prev(self.cursor.col, n);
        self.goto(self.cursor.row, col);
    }
}
