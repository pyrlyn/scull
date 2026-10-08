//! Literal search over the scrollback and the screen. Separate from the
//! selection because it reads text the user did not mark.
//!
//! Nothing but the pattern and the current match is kept: matches are
//! found again on every call, so they never go stale under new output, and
//! the frame searches only the viewport. Rows are read one at a time into
//! a window that carries the tail of a soft-wrapped row on to the next, so
//! a match may cross a wrap while the window never outgrows a row plus the
//! pattern. Case folding maps each character to its lowercase form, the
//! pattern and the text alike.

use std::ops::ControlFlow;

use scull_grid::Row;

use crate::frame::Mark;
use crate::selection::{PADDING, Point};
use crate::terminal::Terminal;
use crate::text::push_cell;

/// Bytes of pattern accepted: a find bar's worth. The window carries this
/// much from row to row, so it bounds the cost of each row searched.
pub const MAX_SEARCH_PATTERN: usize = 256;

/// Matches counted or listed at most.
pub const MAX_SEARCH_MATCHES: usize = 10_000;

/// Lines a backward search scans per step before looking further back.
const BACK_STEP: u64 = 256;

/// One match: its first and last cell, inclusive (a wide character's spacer
/// included), in absolute lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Match {
    /// First cell.
    pub start: Point,
    /// Last cell.
    pub end: Point,
}

#[derive(Clone, Debug)]
pub(crate) struct Search {
    /// The pattern, folded unless the search is case-sensitive.
    pattern: String,
    /// Characters in `pattern`.
    chars: usize,
    fold: bool,
    current: Option<Match>,
}

impl Search {
    pub(crate) fn forget_current(&mut self) {
        self.current = None;
    }
}

/// Text read from consecutive rows, each character with the cell it came
/// from: its byte offset, the cell and the last column the cell covers.
#[derive(Default)]
struct Window {
    text: String,
    cells: Vec<(usize, Point, u16)>,
    cell: String,
}

impl Window {
    /// Keeps the last `keep` characters only.
    fn keep_last(&mut self, keep: usize) {
        let from = self.cells.len().saturating_sub(keep);
        let byte = self.cells.get(from).map_or(self.text.len(), |c| c.0);
        self.text.drain(..byte);
        self.cells.drain(..from);
        for c in &mut self.cells {
            c.0 -= byte;
        }
    }

    /// The match covering bytes `s..e`.
    fn span(&self, s: usize, e: usize) -> Option<Match> {
        let first = self.cells.partition_point(|c| c.0 <= s).checked_sub(1)?;
        let last = self.cells.partition_point(|c| c.0 < e).checked_sub(1)?;
        let (_, start, _) = *self.cells.get(first)?;
        let (_, end, end_col) = *self.cells.get(last)?;
        Some(Match {
            start,
            end: Point {
                col: end_col,
                ..end
            },
        })
    }
}

impl Terminal {
    /// Searches for `pattern`, case-sensitive or not, replacing any earlier
    /// search. An empty pattern, or one longer than [`MAX_SEARCH_PATTERN`]
    /// bytes, clears the search and answers false.
    pub fn search_set(&mut self, pattern: &str, case_sensitive: bool) -> bool {
        self.search = None;
        if pattern.is_empty() || pattern.len() > MAX_SEARCH_PATTERN {
            return false;
        }
        let pattern: String = if case_sensitive {
            pattern.to_owned()
        } else {
            pattern.chars().flat_map(char::to_lowercase).collect()
        };
        self.search = Some(Search {
            chars: pattern.chars().count(),
            pattern,
            fold: !case_sensitive,
            current: None,
        });
        true
    }

    /// Ends the search.
    pub fn search_clear(&mut self) {
        self.search = None;
    }

    /// The match last stepped to, which the frame marks as current.
    pub fn search_current(&self) -> Option<Match> {
        self.search.as_ref().and_then(|s| s.current)
    }

    /// Steps to the first match after `from` (by default after the current
    /// match, or from the top of the viewport), wrapping round to the
    /// oldest line, and scrolls the viewport to show it.
    pub fn search_next(&mut self, from: Option<Point>) -> Option<Match> {
        let s = self.search.as_ref()?;
        let (from, inclusive) = origin(s, from, self.viewport_point(0, 0));
        let found = self
            .find_forward(s, from, inclusive, self.last_line())
            .or_else(|| self.find_forward(s, self.first_line_point(), true, from.line));
        self.step_to(found)
    }

    /// Steps to the last match before `from` (by default before the current
    /// match, or from the bottom of the viewport), wrapping round to the
    /// screen's bottom, and scrolls the viewport to show it.
    pub fn search_prev(&mut self, from: Option<Point>) -> Option<Match> {
        let s = self.search.as_ref()?;
        let bottom = self.viewport_point(u16::MAX, u16::MAX);
        let (from, inclusive) = origin(s, from, bottom);
        let end = Point {
            line: self.last_line(),
            col: u16::MAX,
        };
        let found = self
            .find_backward(s, from, inclusive)
            .or_else(|| self.find_backward(s, end, true));
        self.step_to(found)
    }

    /// Appends every match, oldest first, up to [`MAX_SEARCH_MATCHES`].
    pub fn search_matches(&self, out: &mut Vec<Match>) {
        let Some(s) = &self.search else {
            return;
        };
        let mut left = MAX_SEARCH_MATCHES;
        self.scan(s, self.first_line(), self.last_line(), |m| {
            out.push(m);
            left -= 1;
            if left == 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
    }

    /// How many matches there are, counting up to [`MAX_SEARCH_MATCHES`].
    pub fn search_count(&self) -> usize {
        let Some(s) = &self.search else {
            return 0;
        };
        let mut n = 0;
        self.scan(s, self.first_line(), self.last_line(), |_| {
            n += 1;
            if n < MAX_SEARCH_MATCHES {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        });
        n
    }

    /// Lays the matches in the viewport over the cells they cover.
    pub(crate) fn search_marks(&self, out: &mut Vec<Mark>) {
        let Some(s) = &self.search else {
            return;
        };
        let top = self.viewport_point(0, 0);
        let bottom = self.viewport_point(u16::MAX, u16::MAX).line;
        let mut left = MAX_SEARCH_MATCHES;
        self.scan(s, top.line, bottom, |m| {
            if m.start.line > bottom {
                return ControlFlow::Break(());
            }
            if m.end < top {
                return ControlFlow::Continue(());
            }
            let mut flags = crate::FrameCell::MATCH;
            if s.current == Some(m) {
                flags |= crate::FrameCell::CURRENT_MATCH;
            }
            for line in m.start.line.max(top.line)..=m.end.line.min(bottom) {
                let start = if line == m.start.line { m.start.col } else { 0 };
                let end = if line == m.end.line {
                    m.end.col
                } else {
                    self.last_col()
                };
                out.push(Mark {
                    row: self.viewport_row(line).unwrap_or_default(),
                    start,
                    end: end.saturating_add(1),
                    flags,
                });
            }
            left -= 1;
            if left == 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
    }

    fn first_line_point(&self) -> Point {
        Point {
            line: self.first_line(),
            col: 0,
        }
    }

    fn step_to(&mut self, found: Option<Match>) -> Option<Match> {
        let m = found?;
        if let Some(s) = &mut self.search {
            s.current = Some(m);
        }
        if self.viewport_row(m.start.line).is_none() {
            // Shown mid-viewport, so the lines around it give context.
            // The screen itself needs no offset; history is clamped to.
            let above = self.screen_top_line().saturating_sub(m.start.line);
            let half = u64::from(self.grid().screen_rows() / 2);
            let offset = if above == 0 {
                0
            } else {
                above.saturating_add(half)
            };
            let target = isize::try_from(offset).unwrap_or(isize::MAX);
            let now = isize::try_from(self.grid().display_offset()).unwrap_or(isize::MAX);
            self.scroll_display(target.saturating_sub(now));
        }
        Some(m)
    }

    fn find_forward(&self, s: &Search, from: Point, inclusive: bool, last: u64) -> Option<Match> {
        let mut hit = None;
        self.scan(s, from.line, last, |m| {
            if m.start > from || (inclusive && m.start == from) {
                hit = Some(m);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        hit
    }

    fn find_backward(&self, s: &Search, from: Point, inclusive: bool) -> Option<Match> {
        let first = self.first_line();
        let mut hi = from.line.min(self.last_line());
        loop {
            let lo = hi.saturating_sub(BACK_STEP - 1).max(first);
            let mut hit = None;
            self.scan(s, lo, hi, |m| {
                if m.start < from || (inclusive && m.start == from) {
                    hit = Some(m);
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            });
            if hit.is_some() || lo <= first {
                return hit;
            }
            hi = lo - 1;
        }
    }

    /// Calls `visit` with each match starting on lines `first..=last` (and
    /// any that begins up to one pattern earlier inside the same soft-wrapped
    /// line, or runs on past `last`), in order, until it breaks.
    fn scan(
        &self,
        s: &Search,
        first: u64,
        last: u64,
        mut visit: impl FnMut(Match) -> ControlFlow<()>,
    ) {
        let keep = s.chars.saturating_sub(1);
        // A wrapped row holds at least half its width in characters.
        let per_row = usize::from(self.grid().cols() / 2).max(1);
        let reach = u64::try_from(keep.div_ceil(per_row)).unwrap_or(u64::MAX);
        let (lo, hi) = (self.first_line(), self.last_line());
        let mut line = first.max(lo);
        for _ in 0..reach {
            match line.checked_sub(1) {
                Some(up) if self.line_row(up).is_some_and(Row::wrapped) => line = up,
                _ => break,
            }
        }
        let mut stop = last.min(hi);
        for _ in 0..reach {
            if stop < hi && self.line_row(stop).is_some_and(Row::wrapped) {
                stop += 1;
            } else {
                break;
            }
        }
        let mut w = Window::default();
        let mut last_end: Option<Point> = None;
        while line <= stop {
            let Some(row) = self.line_row(line) else {
                break;
            };
            w.keep_last(keep);
            self.read_row(row, line, s.fold, &mut w);
            let skip = last_end.map_or(0, |e| w.cells.partition_point(|c| c.1 <= e));
            let mut pos = w.cells.get(skip).map_or(w.text.len(), |c| c.0);
            while let Some(at) = w.text.get(pos..).and_then(|t| t.find(&s.pattern)) {
                let (start, end) = (pos + at, pos + at + s.pattern.len());
                pos = end;
                let Some(m) = w.span(start, end) else {
                    continue;
                };
                last_end = Some(m.end);
                if visit(m).is_break() {
                    return;
                }
            }
            if !row.wrapped() {
                w.keep_last(0);
            }
            line += 1;
        }
    }

    /// Appends `row`'s characters to the window; a row that ends its line
    /// loses its trailing blanks, as copied text does.
    fn read_row(&self, row: &Row, line: u64, fold: bool, w: &mut Window) {
        let grid = self.grid();
        let first = w.cells.len();
        for (col, cell) in (0..=u16::MAX).zip(row.cells()) {
            if cell.flags().intersects(PADDING) {
                continue;
            }
            w.cell.clear();
            push_cell(grid, cell, &mut w.cell);
            let wide = cell.flags().contains(scull_grid::CellFlags::WIDE);
            let last_col = col.saturating_add(u16::from(wide));
            let at = Point { line, col };
            for c in w.cell.chars() {
                let folded = fold.then(|| c.to_lowercase());
                for c in folded.into_iter().flatten().chain((!fold).then_some(c)) {
                    w.cells.push((w.text.len(), at, last_col));
                    w.text.push(c);
                }
            }
        }
        if !row.wrapped() {
            while w.cells.len() > first && w.text.ends_with(' ') {
                w.cells.pop();
                w.text.pop();
            }
        }
    }
}

/// Where a step starts: `from`, else the current match (excluded), else
/// `fallback` (included).
fn origin(s: &Search, from: Option<Point>, fallback: Point) -> (Point, bool) {
    match (from, s.current) {
        (Some(p), _) => (p, false),
        (None, Some(m)) => (m.start, false),
        (None, None) => (fallback, true),
    }
}

#[cfg(test)]
mod tests;
