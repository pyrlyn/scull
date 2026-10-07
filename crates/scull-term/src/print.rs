//! Printing: code points into grapheme clusters, clusters into one or two
//! cells, and autowrap. Separate because it is the hot path and the one
//! place where Unicode width meets the grid.
//!
//! Autowrap follows DEC STD 070 and xterm (ctlseqs, DECAWM): writing the
//! last column of the line leaves the cursor there with a pending wrap, and
//! only the next printable character wraps, to the left margin of the next
//! line. A wide character that does not fit at the end of a line leaves a
//! leading-spacer cell behind and wraps whole, as foot and Alacritty do.
//!
//! A code point that does not start a new grapheme cluster (UAX #29), or
//! that is zero cells wide, joins the cell left of the cursor. Under mode
//! 2027 the joined cluster may change width (VS15, VS16, emoji ZWJ
//! sequences) and the cursor moves with it.

use scull_grid::{Cell, CellFlags, Content};
use scull_unicode::{cluster_width, width};

use crate::state::State;

const WIDE: u8 = 2;

impl State {
    /// A run of printable text from the parser.
    pub(crate) fn print_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.print_char(ch);
        }
    }

    pub(crate) fn print_char(&mut self, ch: char) {
        let ch = self.charsets.map(ch);
        let starts_cluster = self.grapheme.next(ch);
        let cells = width(ch, self.width);
        if starts_cluster && cells > 0 {
            self.place(ch, cells);
        } else {
            self.join(ch);
        }
    }

    /// Writes a new cluster at the cursor and advances past it.
    fn place(&mut self, ch: char, cells: u8) {
        if self.cursor.pending_wrap && self.modes.autowrap {
            self.wrap();
        }
        let span = u16::from(cells);
        if self.cursor.col.saturating_add(span - 1) > self.line_end() {
            if self.modes.autowrap {
                let spacer = self.blank().with_flags(CellFlags::LEADING_SPACER);
                let col = self.cursor.col;
                if let Some(row) = self.cursor_row() {
                    // Fails only past the row, which the cursor never is.
                    let _ = row.set(col, spacer);
                }
                self.wrap();
            } else {
                self.cursor.col = self.line_end().saturating_sub(span - 1);
            }
            if self.cursor.col.saturating_add(span - 1) > self.line_end() {
                // The margins are narrower than the character.
                return;
            }
        }
        if self.modes.insert {
            self.insert_chars(span);
        }
        let style = self.pen.text_id(&mut self.grid);
        let col = self.cursor.col;
        if let Some(row) = self.cursor_row() {
            let _ = row.put(col, Cell::char(ch, style), cells == WIDE);
        }
        self.advance(col, span);
        self.last_char = Some(ch);
    }

    /// Moves the cursor past a cluster written at `col`, entering the
    /// pending wrap when it filled the line.
    fn advance(&mut self, col: u16, span: u16) {
        let end = self.line_end();
        let next = col.saturating_add(span);
        if next > end {
            self.cursor.col = end;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = next;
            self.cursor.pending_wrap = false;
        }
    }

    /// Autowrap: marks the row as continued and starts the next line at the
    /// left margin, scrolling if the cursor is on the bottom margin.
    fn wrap(&mut self) {
        if let Some(row) = self.cursor_row() {
            row.set_wrapped(true);
        }
        self.index();
        self.cursor.col = self.margins.left;
    }

    /// Adds `ch` to the cluster in the cell left of the cursor. With no
    /// cluster there to join, the code point is dropped, as it has no cell.
    fn join(&mut self, ch: char) {
        let Some(col) = self.cluster_col() else {
            return;
        };
        let Some(cell) = self
            .grid
            .screen_row(self.cursor.row)
            .and_then(|row| row.cell(col))
        else {
            return;
        };
        let mut text = String::new();
        match cell.content() {
            Content::Empty => return,
            Content::Char(c) => text.push(c),
            Content::Cluster(id) => text.push_str(self.grid.clusters().get(id).unwrap_or_default()),
        }
        text.push(ch);
        // Past the cluster cap the extra code point is dropped.
        let Ok(id) = self.grid.intern_cluster(&text) else {
            return;
        };
        let was_wide = cell.flags().contains(CellFlags::WIDE);
        let fits_wide = was_wide || col < self.line_end();
        let wide = fits_wide && cluster_width(&text, self.width, self.policy()) == WIDE;
        if let Some(row) = self.cursor_row() {
            let _ = row.put(col, Cell::cluster(id, cell.style()), wide);
        }
        if wide != was_wide {
            self.advance(col, if wide { u16::from(WIDE) } else { 1 });
        }
    }

    /// The head cell of the cluster just left of the cursor: under the
    /// cursor while a wrap is pending, otherwise one or two columns left.
    fn cluster_col(&self) -> Option<u16> {
        let cursor = self.cursor;
        let col = if cursor.pending_wrap {
            cursor.col
        } else {
            cursor.col.checked_sub(1)?
        };
        let cell = self.grid.screen_row(cursor.row)?.cell(col)?;
        if cell.flags().contains(CellFlags::SPACER) {
            col.checked_sub(1)
        } else {
            Some(col)
        }
    }

    /// REP: the last printed character `n` more times, capped at one screen
    /// of cells so a short sequence cannot demand unbounded work.
    pub(crate) fn repeat(&mut self, n: u16) {
        let Some(ch) = self.last_char else {
            return;
        };
        let cap = usize::from(self.cols()) * usize::from(self.rows());
        for _ in 0..usize::from(n).min(cap) {
            self.end_cluster();
            self.print_char(ch);
        }
    }
}
