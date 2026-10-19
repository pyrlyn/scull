//! The selection: cells the user marked, kept in absolute lines (the lines
//! image placements are anchored to, `images.rs`) so it stays on its text
//! while output scrolls and the viewport moves. Separate from the frame,
//! which only shows it; copying reads the grid here.
//!
//! The selection is dropped, never repaired, once the text under it may
//! have changed: a row it covers was rewritten or replaced, its first or
//! last line left the ring, the screens switched or the terminal was
//! resized. Alacritty clears its selection on writes into it the same way.

use scull_grid::{CellFlags, Content, Row};

use crate::damage::RowStamp;
use crate::frame::Mark;
use crate::terminal::Terminal;
use crate::text::{push_cell, trim_line};

/// Characters that end a word besides blanks. Alacritty's default
/// `semantic_escape_chars`, so paths, URLs, options and `key=value` pairs
/// select whole while brackets, quotes and pipes do not stick to them.
pub const WORD_SEPARATORS: &str = ",\u{2502}`|:\"'()[]{}<>";

/// Cells a word selection walks each way at most. Only a program prints a
/// longer word, and walking it must not stall the UI thread.
pub const MAX_WORD_CELLS: usize = 4096;

/// Bytes of selected text handed out at most; more is cut at a character
/// boundary. A full ring of wide rows would otherwise be gigabytes.
pub const MAX_SELECTION_BYTES: usize = 16 << 20;

/// Cells a selection or match is bounded by: absolute lines, counted like
/// [`Terminal::screen_top_line`], and columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    /// Absolute line.
    pub line: u64,
    /// Column.
    pub col: u16,
}

/// What a selection grows by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Cell by cell, in reading order (a drag).
    Cell,
    /// Whole words, ended by blanks and [`WORD_SEPARATORS`] (a double click).
    Word,
    /// Whole lines, soft wraps included (a triple click).
    Line,
    /// A rectangle of columns (a drag with a modifier).
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Word,
    Blank,
    Separator,
    /// The half of a wide character behind its head, or the padding before
    /// a wide character that wrapped: part of whatever surrounds it.
    Padding,
}

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    kind: SelectionKind,
    anchor: Point,
    start: Point,
    end: Point,
    alt: bool,
    /// The rows it covers that output can still change: the screen's and
    /// both ends'. History rows in between change only by leaving the
    /// ring, and the first one leaves first.
    stamps: Vec<(u64, RowStamp)>,
}

impl Selection {
    /// The columns selected on `line`, inclusive.
    fn cols_on(&self, line: u64, last_col: u16) -> (u16, u16) {
        if self.kind == SelectionKind::Block {
            return (self.start.col, self.end.col);
        }
        let from = if line == self.start.line {
            self.start.col
        } else {
            0
        };
        let to = if line == self.end.line {
            self.end.col
        } else {
            last_col
        };
        (from, to)
    }
}

impl Terminal {
    /// The cell at viewport row `row` and column `col`, clamped to the
    /// viewport, as an absolute point.
    pub fn viewport_point(&self, row: u16, col: u16) -> Point {
        let grid = self.grid();
        let back = u64::try_from(grid.display_offset()).unwrap_or(u64::MAX);
        let row = row.min(grid.screen_rows().saturating_sub(1));
        Point {
            line: self
                .screen_top_line()
                .saturating_sub(back)
                .saturating_add(u64::from(row)),
            col: col.min(self.last_col()),
        }
    }

    /// The viewport row showing absolute `line`, if it is shown.
    pub fn viewport_row(&self, line: u64) -> Option<u16> {
        let top = self.viewport_point(0, 0).line;
        u16::try_from(line.checked_sub(top)?)
            .ok()
            .filter(|&r| r < self.grid().screen_rows())
    }

    /// Starts a selection of `kind` at `at`, replacing any other. Until it
    /// is extended it covers the cell, word or line at `at`; a host clears
    /// it on a click that did not drag. A point outside the ring selects
    /// nothing.
    pub fn select_start(&mut self, kind: SelectionKind, at: Point) {
        self.selection = None;
        if self.line_row(at.line).is_none() {
            return;
        }
        let at = Point {
            col: at.col.min(self.last_col()),
            ..at
        };
        let mut sel = Selection {
            kind,
            anchor: at,
            start: at,
            end: at,
            alt: self.is_alt_screen(),
            stamps: Vec::new(),
        };
        self.resolve(&mut sel, at);
        self.selection = Some(sel);
    }

    /// Moves the selection's free end to `to`, clamped to the ring.
    pub fn select_extend(&mut self, to: Point) {
        let Some(mut sel) = self.selection.take() else {
            return;
        };
        let to = Point {
            line: to.line.clamp(self.first_line(), self.last_line()),
            col: to.col.min(self.last_col()),
        };
        self.resolve(&mut sel, to);
        self.selection = Some(sel);
    }

    /// Drops the selection.
    pub fn select_clear(&mut self) {
        self.selection = None;
    }

    /// The selection's kind and its first and last cell (inclusive, a wide
    /// character's spacer included). For a block the points are the top
    /// left and bottom right corners.
    pub fn selection(&self) -> Option<(SelectionKind, Point, Point)> {
        self.selection.as_ref().map(|s| (s.kind, s.start, s.end))
    }

    /// Appends the selected text to `out`: soft-wrapped rows joined, hard
    /// line ends (and block rows) as `\n`, wide characters once, blank and
    /// concealed cells as spaces, trailing spaces trimmed per line. Cut at
    /// [`MAX_SELECTION_BYTES`].
    pub fn selection_text(&self, out: &mut String) {
        self.selection_text_capped(out, MAX_SELECTION_BYTES);
    }

    fn selection_text_capped(&self, out: &mut String, cap: usize) {
        let Some(sel) = &self.selection else {
            return;
        };
        let (grid, base, last_col) = (self.grid(), out.len(), self.last_col());
        for line in sel.start.line..=sel.end.line {
            if out.len() - base > cap {
                break;
            }
            let Some(row) = self.line_row(line) else {
                continue;
            };
            let (from, to) = sel.cols_on(line, last_col);
            let start = out.len();
            let cells = row.cells().skip(usize::from(from));
            for cell in cells.take(usize::from(to.saturating_sub(from)) + 1) {
                if !cell.flags().intersects(PADDING) {
                    push_cell(grid, cell, out);
                }
            }
            if line == sel.end.line {
                trim_line(out, start);
            } else if sel.kind == SelectionKind::Block || !row.wrapped() {
                trim_line(out, start);
                out.push('\n');
            }
        }
        if out.len() - base > cap {
            let mut cut = base + cap;
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
        }
    }

    /// Lays the selection over the viewport rows it covers.
    pub(crate) fn selection_marks(&self, out: &mut Vec<Mark>) {
        let Some(sel) = &self.selection else {
            return;
        };
        let top = self.viewport_point(0, 0).line;
        for row in 0..self.grid().screen_rows() {
            let line = top.saturating_add(u64::from(row));
            if (sel.start.line..=sel.end.line).contains(&line) {
                let (from, to) = sel.cols_on(line, self.last_col());
                out.push(Mark {
                    row,
                    start: from,
                    end: to.saturating_add(1),
                    flags: crate::FrameCell::SELECTED,
                });
            }
        }
    }

    /// Drops the selection when the text under it may have changed; called
    /// after every feed.
    pub(crate) fn check_selection(&mut self) {
        let keep = self.selection.as_ref().is_some_and(|s| {
            s.alt == self.is_alt_screen()
                && s.stamps
                    .iter()
                    .all(|&(line, stamp)| self.line_row(line).map(RowStamp::of) == Some(stamp))
        });
        if !keep {
            self.selection = None;
        }
    }

    /// The row at absolute `line`, if the ring still holds it.
    pub(crate) fn line_row(&self, line: u64) -> Option<&Row> {
        let at = line.checked_sub(self.first_line())?;
        self.grid().row(usize::try_from(at).ok()?)
    }

    /// The oldest line the ring holds.
    pub(crate) fn first_line(&self) -> u64 {
        self.state.images.evicted
    }

    /// The bottom screen row's line.
    pub(crate) fn last_line(&self) -> u64 {
        self.state.line_of(self.state.last_row())
    }

    pub(crate) fn last_col(&self) -> u16 {
        self.state.last_col()
    }

    /// Sets the selection's bounds from its anchor and `focus`, and stamps
    /// the rows that could change under it.
    fn resolve(&self, sel: &mut Selection, focus: Point) {
        let (a, b) = if sel.anchor <= focus {
            (sel.anchor, focus)
        } else {
            (focus, sel.anchor)
        };
        let last_col = self.last_col();
        (sel.start, sel.end) = match sel.kind {
            SelectionKind::Cell => (self.head(a), self.tail(b)),
            SelectionKind::Word => (self.word_edge(a, false), self.word_edge(b, true)),
            SelectionKind::Line => (
                Point {
                    line: self.logical_edge(a.line, false),
                    col: 0,
                },
                Point {
                    line: self.logical_edge(b.line, true),
                    col: last_col,
                },
            ),
            SelectionKind::Block => (
                Point {
                    line: a.line,
                    col: a.col.min(b.col),
                },
                Point {
                    line: b.line,
                    col: a.col.max(b.col),
                },
            ),
        };
        let screen = sel.start.line.max(self.screen_top_line())..=sel.end.line;
        sel.stamps.clear();
        for line in [sel.start.line, sel.end.line].into_iter().chain(screen) {
            if let Some(row) = self.line_row(line) {
                sel.stamps.push((line, RowStamp::of(row)));
            }
        }
    }

    fn flags(&self, p: Point) -> CellFlags {
        self.line_row(p.line)
            .and_then(|r| r.cell(p.col))
            .map(|c| c.flags())
            .unwrap_or_default()
    }

    /// `p`, or the wide character's head when `p` is its spacer.
    fn head(&self, p: Point) -> Point {
        if p.col > 0 && self.flags(p).contains(CellFlags::SPACER) {
            Point {
                col: p.col - 1,
                ..p
            }
        } else {
            p
        }
    }

    /// `p`, or its spacer when `p` is a wide character's head.
    fn tail(&self, p: Point) -> Point {
        if p.col < self.last_col() && self.flags(p).contains(CellFlags::WIDE) {
            Point {
                col: p.col + 1,
                ..p
            }
        } else {
            p
        }
    }

    fn class(&self, p: Point) -> Class {
        let Some(cell) = self.line_row(p.line).and_then(|r| r.cell(p.col)) else {
            return Class::Blank;
        };
        if cell.flags().intersects(PADDING) {
            return Class::Padding;
        }
        match cell.content() {
            Content::Empty => Class::Blank,
            Content::Char(c) if c.is_whitespace() => Class::Blank,
            Content::Char(c) if WORD_SEPARATORS.contains(c) => Class::Separator,
            _ => Class::Word,
        }
    }

    /// The next cell in reading order, across a soft wrap but not a hard
    /// line end.
    fn step(&self, p: Point, forward: bool) -> Option<Point> {
        let last_col = self.last_col();
        if forward {
            if p.col < last_col {
                return Some(Point {
                    col: p.col + 1,
                    ..p
                });
            }
            let next = p.line.checked_add(1)?;
            let wraps = self.line_row(p.line)?.wrapped() && self.line_row(next).is_some();
            wraps.then_some(Point { line: next, col: 0 })
        } else {
            if p.col > 0 {
                return Some(Point {
                    col: p.col - 1,
                    ..p
                });
            }
            let up = p.line.checked_sub(1)?;
            self.line_row(up)?.wrapped().then_some(Point {
                line: up,
                col: last_col,
            })
        }
    }

    /// The first (or, `forward`, the last) cell of the word at `p`. A
    /// separator is a word of its own; a run of blanks is one word.
    fn word_edge(&self, p: Point, forward: bool) -> Point {
        let p = self.head(p);
        let class = match self.class(p) {
            Class::Padding => Class::Blank,
            c => c,
        };
        let mut edge = p;
        if class != Class::Separator {
            for _ in 0..MAX_WORD_CELLS {
                let Some(next) = self.step(edge, forward) else {
                    break;
                };
                let c = self.class(next);
                if c != class && c != Class::Padding {
                    break;
                }
                edge = next;
            }
        }
        if forward { self.tail(edge) } else { edge }
    }

    /// The first (or, `forward`, the last) line of the soft-wrapped line
    /// through `line`.
    fn logical_edge(&self, mut line: u64, forward: bool) -> u64 {
        loop {
            let next = if forward {
                line.checked_add(1)
                    .filter(|_| self.line_row(line).is_some_and(Row::wrapped))
            } else {
                line.checked_sub(1)
                    .filter(|&up| self.line_row(up).is_some_and(Row::wrapped))
            };
            match next {
                Some(n) if self.line_row(n).is_some() => line = n,
                _ => return line,
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// Cells that hold no text of their own.
pub(crate) const PADDING: CellFlags = CellFlags::SPACER.union(CellFlags::LEADING_SPACER);
