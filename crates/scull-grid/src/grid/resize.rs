//! Resizing the grid: rewrap (or truncate) every row, then lay the result
//! out as history plus screen and rebuild the ring. A child of the grid
//! module because it replaces the ring wholesale; the per-line work lives in
//! `reflow` so it is tested without a ring.

use crate::GridError;
use crate::cell::Cell;
use crate::grid::{Grid, MAX_RING_ROWS};
use crate::reflow::{self, Spot};
use crate::row::{Row, RowId};

/// How rows meet a new width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reflow {
    /// Join soft-wrapped rows into lines and cut them at the new width: the
    /// primary screen and its scrollback.
    Rewrap,
    /// Keep every row as it is, cutting or padding its tail: the alt screen,
    /// whose application redraws on `SIGWINCH` anyway.
    Truncate,
}

/// A position that must follow its cell through a resize: the cursor, the
/// saved cursor, selection ends, a search match.
///
/// `row` is a logical grid row (0 is the oldest history row,
/// [`Grid::history_len`] the top of the screen), so points in scrollback and
/// on the screen are expressed alike; a screen position `(r, c)` is
/// `TrackPoint::new(grid.history_len() + r, c)` before the resize and
/// `point.row - grid.history_len()` after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackPoint {
    /// Logical row.
    pub row: usize,
    /// Column.
    pub col: u16,
    /// Set by the last resize when the point's cell did not survive (history
    /// evicted it, truncation cut it, the row it was on was dropped) and the
    /// point was clamped to the nearest place still in the grid. A
    /// selection whose end is clamped should be cleared.
    pub clamped: bool,
}

impl TrackPoint {
    /// A point at logical `row`, column `col`.
    pub const fn new(row: usize, col: u16) -> Self {
        Self {
            row,
            col,
            clamped: false,
        }
    }
}

/// Index of the cursor in the internal point list; the viewport, when
/// scrolled back, follows it, then the caller's points.
const CURSOR: usize = 0;

impl Grid {
    /// Resizes to `cols` x `screen_rows` and moves `cursor` and every point
    /// in `points` with its cell.
    ///
    /// With [`Reflow::Rewrap`] and a new width, history and screen are
    /// rewrapped together in one pass: soft-wrapped rows (see
    /// [`Row::wrapped`], set by whoever wraps) are joined into lines and cut
    /// again, wide characters move whole to the next row behind a
    /// `LEADING_SPACER`, styles, clusters and links travel with their cells.
    /// A row that starts where an old row started keeps that row's id, and
    /// its generation when its content is unchanged; every other row gets a
    /// fresh id. A frame must still repaint everything after a resize.
    ///
    /// Then trailing blank rows below every point are dropped, the cursor's
    /// row is kept on the screen (rows that do not fit below it are lost),
    /// the screen is the last `screen_rows` rows, and history keeps at most
    /// the configured scrollback, evicting the oldest rows. A scrolled-back
    /// viewport stays on the line at its top. The cursor ends on the screen;
    /// other points (a saved cursor among them) may end in history, and the
    /// caller clamps those that must be on the screen.
    ///
    /// Contract for the C ABI (T10): during an interactive resize the host
    /// pauses PTY reads and keeps drawing the last frame, then calls this
    /// once with the final size, resizes the PTY and resumes reads. A
    /// rewrap per drag step would cost one pass over all history each step
    /// and, at a narrow intermediate width, could push lines past the
    /// history cap for good.
    ///
    /// The sizes come from the UI and are untrusted: zero is refused, a
    /// point outside the grid is clamped into it first and reported as
    /// clamped. The same size is a no-op.
    pub fn resize(
        &mut self,
        cols: u16,
        screen_rows: u16,
        reflow: Reflow,
        cursor: &mut TrackPoint,
        points: &mut [TrackPoint],
    ) -> Result<(), GridError> {
        if cols == 0 || screen_rows == 0 {
            return Err(GridError::Empty);
        }
        let total = self.history + usize::from(self.screen_rows);
        let viewport = (self.display_offset > 0)
            .then(|| TrackPoint::new(self.history.saturating_sub(self.display_offset), 0));
        let mut spots: Vec<Spot> = std::iter::once(*cursor)
            .chain(viewport)
            .chain(points.iter().copied())
            .map(|p| clamp_in(p, total, self.cols))
            .collect();
        if (cols, screen_rows) != (self.cols, self.screen_rows) {
            let mut old = self.take_rows(total);
            let rows = if reflow == Reflow::Rewrap && cols != self.cols {
                reflow::rewrap(&mut old, cols, &mut spots, &mut self.next_id)
            } else {
                truncate(old, cols, &mut spots)
            };
            self.lay_out(rows, cols, screen_rows, &mut spots);
            if viewport.is_some() {
                let top = spots.get(CURSOR + 1).map_or(0, |s| s.row);
                self.display_offset = self.history.saturating_sub(top);
            }
        }
        let mut done = spots.into_iter().map(|s| TrackPoint {
            row: s.row,
            col: s.col,
            clamped: s.lost,
        });
        if let Some(c) = done.next() {
            *cursor = c;
        }
        if viewport.is_some() {
            done.next();
        }
        for (p, s) in points.iter_mut().zip(done) {
            *p = s;
        }
        Ok(())
    }

    /// Moves every live row out of the ring, oldest first.
    fn take_rows(&mut self, total: usize) -> Vec<Row> {
        let mut old = Vec::with_capacity(total);
        for i in 0..total {
            let at = self.phys(i);
            let row = match self.rows.get_mut(at) {
                Some(row) => std::mem::replace(row, Row::new(RowId::default(), 0, Cell::EMPTY)),
                // A missing slot is a broken invariant; a blank row keeps
                // the point rows aligned instead of panicking.
                None => Row::new(self.fresh_id(), self.cols, Cell::EMPTY),
            };
            old.push(row);
        }
        self.rows = Vec::new();
        old
    }

    /// Splits `rows` into history and screen and installs them as the ring.
    fn lay_out(&mut self, mut rows: Vec<Row>, cols: u16, screen_rows: u16, spots: &mut [Spot]) {
        let screen = usize::from(screen_rows);
        let lowest_point = spots.iter().map(|s| s.row).max().unwrap_or(0);
        let blank_tail = rows
            .iter()
            .skip(lowest_point + 1)
            .rev()
            .take_while(|r| r.is_blank())
            .count();
        // Blank rows go only where they would push content into history.
        let keep = rows
            .len()
            .saturating_sub(blank_tail)
            .max(rows.len().min(screen));
        rows.truncate(keep);
        let cursor_row = spots.get(CURSOR).map_or(0, |s| s.row);
        let fits = cursor_row + screen;
        if rows.len() > fits {
            rows.truncate(fits);
            self.released_since_sweep += keep - fits;
            for s in spots.iter_mut().filter(|s| s.row >= fits) {
                *s = Spot {
                    row: fits - 1,
                    col: cols - 1,
                    lost: true,
                };
            }
        }
        while rows.len() < screen {
            let id = self.fresh_id();
            rows.push(Row::new(id, cols, Cell::EMPTY));
        }
        let limit = self.scrollback.min(MAX_RING_ROWS - screen);
        let evicted = (rows.len() - screen).saturating_sub(limit);
        rows.drain(..evicted);
        self.released_since_sweep += evicted;
        for s in spots.iter_mut() {
            if s.row < evicted {
                *s = Spot {
                    row: 0,
                    col: 0,
                    lost: true,
                };
            } else {
                s.row -= evicted;
            }
        }
        self.history = rows.len() - screen;
        self.history_limit = limit;
        self.mask = (screen + limit).next_power_of_two() - 1;
        self.zero = 0;
        self.rows = rows;
        self.cols = cols;
        self.screen_rows = screen_rows;
        self.display_offset = 0;
    }
}

/// A caller's point moved inside a grid of `total` rows and `cols` columns.
fn clamp_in(p: TrackPoint, total: usize, cols: u16) -> Spot {
    let row = p.row.min(total - 1);
    let col = p.col.min(cols - 1);
    Spot {
        row,
        col,
        lost: (row, col) != (p.row, p.col),
    }
}

/// Every row cut or padded to `cols`; a point whose cell changed is lost.
fn truncate(mut rows: Vec<Row>, cols: u16, spots: &mut [Spot]) -> Vec<Row> {
    let before: Vec<Option<Cell>> = spots
        .iter()
        .map(|s| rows.get(s.row).and_then(|r| r.cell(s.col)))
        .collect();
    for row in &mut rows {
        row.set_cols(cols);
    }
    for (s, was) in spots.iter_mut().zip(before) {
        let col = s.col.min(cols - 1);
        let now = rows.get(s.row).and_then(|r| r.cell(col));
        s.lost |= col != s.col || now != was;
        s.col = col;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellFlags, Content};
    use crate::row::LinkId;
    use crate::style::StyleId;
    use proptest::prelude::*;

    const BIG_HISTORY: usize = 1000;
    const RED: StyleId = StyleId(1);

    /// Test fixture: code points from U+3000 up take two cells.
    fn is_wide(c: char) -> bool {
        c >= '\u{3000}'
    }

    fn put_char(g: &mut Grid, r: u16, c: u16, cell: Cell, wide: bool) {
        g.screen_row_mut(r).unwrap().put(c, cell, wide).unwrap();
    }

    /// Writes like a terminal with deferred autowrap from the screen's top
    /// left: `\n` starts a line, a wide character that does not fit leaves a
    /// `LEADING_SPACER`. Returns where the cursor ends, clamped to the row.
    fn write_styled(g: &mut Grid, text: &[(char, StyleId)]) -> TrackPoint {
        let (mut r, mut c) = (0u16, 0u16);
        for &(ch, style) in text {
            if ch == '\n' {
                newline(g, &mut r);
                c = 0;
                continue;
            }
            let w = if is_wide(ch) { 2 } else { 1 };
            let cols = g.cols();
            if c + w > cols {
                let row = g.screen_row_mut(r).unwrap();
                if w == 2 && c + 1 == cols {
                    row.set(c, Cell::EMPTY.with_flags(CellFlags::LEADING_SPACER))
                        .unwrap();
                }
                row.set_wrapped(true);
                newline(g, &mut r);
                c = 0;
            }
            if w == 2 && g.cols() < 2 {
                continue;
            }
            put_char(g, r, c, Cell::char(ch, style), w == 2);
            c += w;
        }
        TrackPoint::new(g.history_len() + usize::from(r), c.min(g.cols() - 1))
    }

    fn write(g: &mut Grid, text: &str) -> TrackPoint {
        let styled: Vec<_> = text.chars().map(|c| (c, StyleId::DEFAULT)).collect();
        write_styled(g, &styled)
    }

    fn newline(g: &mut Grid, r: &mut u16) {
        if *r + 1 == g.screen_rows() {
            g.scroll_up(1, Cell::EMPTY);
        } else {
            *r += 1;
        }
    }

    fn total(g: &Grid) -> usize {
        g.history_len() + usize::from(g.screen_rows())
    }

    /// Each row as text: `.` empty, `_` spacer, `>` leading spacer, a
    /// trailing `\` for a soft wrap; trailing empties trimmed.
    fn rows(g: &Grid) -> Vec<String> {
        (0..total(g))
            .map(|i| {
                let row = g.row(i).unwrap();
                let mut s: String = row
                    .cells()
                    .map(|c| {
                        let f = c.flags();
                        match c.content() {
                            Content::Char(ch) => ch,
                            _ if f.contains(CellFlags::SPACER) => '_',
                            _ if f.contains(CellFlags::LEADING_SPACER) => '>',
                            _ => '.',
                        }
                    })
                    .collect();
                s.truncate(s.trim_end_matches('.').len());
                if row.wrapped() {
                    s.push('\\');
                }
                s
            })
            .collect()
    }

    /// Logical lines, joined across soft wraps, without trailing blank lines.
    fn lines(g: &Grid) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for i in 0..total(g) {
            let row = g.row(i).unwrap();
            for c in row.cells() {
                if c.flags()
                    .intersects(CellFlags::SPACER | CellFlags::LEADING_SPACER)
                {
                    continue;
                }
                cur.push(match c.content() {
                    Content::Char(ch) => ch,
                    _ => ' ',
                });
            }
            if !row.wrapped() {
                out.push(cur.trim_end().to_owned());
                cur.clear();
            }
        }
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        out
    }

    fn cell_at(g: &Grid, p: TrackPoint) -> Option<Cell> {
        g.row(p.row)?.cell(p.col)
    }

    fn resize(
        g: &mut Grid,
        cols: u16,
        screen: u16,
        cursor: &mut TrackPoint,
        points: &mut [TrackPoint],
    ) {
        g.resize(cols, screen, Reflow::Rewrap, cursor, points)
            .unwrap();
    }

    fn ch(c: char) -> Option<Content> {
        Some(Content::Char(c))
    }

    #[test]
    fn shrink_then_grow_restores_every_row_and_the_cursor() {
        let mut g = Grid::new(10, 4, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "hello world\nsecond line here\nx");
        let original = rows(&g);
        let at = cursor;
        assert_eq!(
            original,
            ["hello worl\\", "d", "second lin\\", "e here", "x"]
        );
        resize(&mut g, 4, 4, &mut cursor, &mut []);
        assert_eq!(
            rows(&g),
            [
                "hell\\", "o wo\\", "rld", "seco\\", "nd l\\", "ine \\", "here", "x"
            ]
        );
        assert_eq!(cursor, TrackPoint::new(7, 1));
        resize(&mut g, 10, 4, &mut cursor, &mut []);
        assert_eq!(rows(&g), original);
        assert_eq!(cursor, at);
    }

    #[test]
    fn wide_character_at_the_wrap_edge_moves_whole_behind_a_leading_spacer() {
        let mut g = Grid::new(6, 3, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcde中f");
        assert_eq!(rows(&g), ["abcde>\\", "中_f", ""]);
        let mut points = [TrackPoint::new(1, 0), TrackPoint::new(1, 1)];
        resize(&mut g, 7, 3, &mut cursor, &mut points);
        assert_eq!(rows(&g), ["abcde中_\\", "f", ""]);
        assert_eq!(points, [TrackPoint::new(0, 5), TrackPoint::new(0, 6)]);
        resize(&mut g, 5, 3, &mut cursor, &mut points);
        assert_eq!(rows(&g), ["abcde\\", "中_f", ""]);
        resize(&mut g, 6, 3, &mut cursor, &mut points);
        assert_eq!(rows(&g), ["abcde>\\", "中_f", ""]);
        assert_eq!(points, [TrackPoint::new(1, 0), TrackPoint::new(1, 1)]);
        assert_eq!(cursor, TrackPoint::new(1, 3));
    }

    #[test]
    fn point_on_a_leading_spacer_moves_to_the_wide_character_as_clamped() {
        let mut g = Grid::new(6, 3, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcde中f");
        let mut points = [TrackPoint::new(0, 5)];
        resize(&mut g, 8, 3, &mut cursor, &mut points);
        assert_eq!(points[0].row, 0);
        assert_eq!(points[0].col, 5);
        assert!(points[0].clamped, "its placeholder cell is gone");
    }

    #[test]
    fn cursor_on_a_wrapped_line_follows_its_cell() {
        let mut g = Grid::new(8, 4, BIG_HISTORY).unwrap();
        write(&mut g, "0123456789AB");
        let mut cursor = TrackPoint::new(1, 2);
        assert_eq!(cell_at(&g, cursor).map(Cell::content), ch('A'));
        resize(&mut g, 5, 4, &mut cursor, &mut []);
        assert_eq!(rows(&g), ["01234\\", "56789\\", "AB", ""]);
        assert_eq!(cursor, TrackPoint::new(2, 0));
        resize(&mut g, 12, 4, &mut cursor, &mut []);
        assert_eq!(cursor, TrackPoint::new(0, 10));
        assert_eq!(cell_at(&g, cursor).map(Cell::content), ch('A'));
    }

    #[test]
    fn cursor_past_the_end_of_a_line_keeps_its_column() {
        let mut g = Grid::new(10, 3, BIG_HISTORY).unwrap();
        write(&mut g, "ab");
        let mut cursor = TrackPoint::new(0, 6);
        resize(&mut g, 4, 3, &mut cursor, &mut []);
        assert_eq!(rows(&g), ["ab\\", "", ""]);
        assert_eq!(cursor, TrackPoint::new(1, 2));
        resize(&mut g, 10, 3, &mut cursor, &mut []);
        assert_eq!(rows(&g), ["ab", "", ""]);
        assert_eq!(cursor, TrackPoint::new(0, 6));
    }

    #[test]
    fn cursor_at_the_end_of_a_full_line_stays_on_its_last_cell() {
        let mut g = Grid::new(4, 3, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcd");
        assert_eq!(cursor, TrackPoint::new(0, 3));
        resize(&mut g, 3, 3, &mut cursor, &mut []);
        assert_eq!(rows(&g), ["abc\\", "d", ""]);
        assert_eq!(cursor, TrackPoint::new(1, 0));
        assert_eq!(cell_at(&g, cursor).map(Cell::content), ch('d'));
    }

    #[test]
    fn points_in_scrollback_follow_their_cells() {
        let mut g = Grid::new(6, 2, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcdefgh\nijkl\nmnopqrst\nuv\nw");
        assert_eq!(g.history_len(), 5);
        let mut points = [TrackPoint::new(1, 1), TrackPoint::new(3, 5)];
        let seen: Vec<_> = points.iter().map(|p| cell_at(&g, *p)).collect();
        assert_eq!(
            seen,
            [
                Some(Cell::char('h', StyleId::DEFAULT)),
                Some(Cell::char('r', StyleId::DEFAULT))
            ]
        );
        for cols in [3, 10, 2, 6] {
            resize(&mut g, cols, 2, &mut cursor, &mut points);
            let now: Vec<_> = points.iter().map(|p| cell_at(&g, *p)).collect();
            assert_eq!(now, seen, "at {cols} columns");
            assert!(points.iter().all(|p| !p.clamped && p.row < g.history_len()));
        }
        assert_eq!(lines(&g), ["abcdefgh", "ijkl", "mnopqrst", "uv", "w"]);
    }

    #[test]
    fn history_cap_reached_during_shrink_evicts_the_oldest_rows() {
        let mut g = Grid::new(6, 2, 2).unwrap();
        let mut cursor = write(&mut g, "aaaaaa\nbbbbbb\ncccccc\ndd");
        assert_eq!(g.history_len(), 2);
        let mut points = [TrackPoint::new(0, 1), TrackPoint::new(1, 4)];
        resize(&mut g, 3, 2, &mut cursor, &mut points);
        assert_eq!(rows(&g), ["bbb", "ccc\\", "ccc", "dd"]);
        assert_eq!(g.history_len(), 2);
        assert!(points[0].clamped && (points[0].row, points[0].col) == (0, 0));
        assert_eq!(
            points[1],
            TrackPoint::new(0, 1),
            "the tail of its line survived"
        );
        assert_eq!(cursor, TrackPoint::new(3, 2));
        assert!(!cursor.clamped);
        assert!(g.rows.len() <= g.capacity());
    }

    #[test]
    fn scrolled_back_viewport_stays_on_the_line_at_its_top() {
        let mut g = Grid::new(8, 3, BIG_HISTORY).unwrap();
        let text: String = ('a'..='p')
            .map(|c| format!("{c}{c}{c}{c}{c}{c}\n"))
            .collect();
        let mut cursor = write(&mut g, &text);
        g.scroll_display(5);
        let top = |g: &Grid| g.visible_row(0).unwrap().cell(0).map(Cell::content);
        let seen = top(&g);
        for cols in [4, 3, 10, 8] {
            resize(&mut g, cols, 3, &mut cursor, &mut []);
            assert_eq!(top(&g), seen, "at {cols} columns");
            assert!(g.display_offset() > 0);
        }
        g.scroll_display(isize::MIN);
        resize(&mut g, 4, 3, &mut cursor, &mut []);
        assert_eq!(
            g.display_offset(),
            0,
            "a viewport at the bottom stays there"
        );
    }

    #[test]
    fn alt_screen_truncates_and_pads_without_rewrap() {
        let mut g = Grid::new(6, 3, 0).unwrap();
        let mut cursor = write(&mut g, "abcd中xyz");
        assert_eq!(rows(&g), ["abcd中_\\", "xyz", ""]);
        let ids: Vec<_> = (0..3).map(|i| g.row(i).unwrap().id()).collect();
        let mut points = [TrackPoint::new(0, 4), TrackPoint::new(1, 2)];
        g.resize(5, 3, Reflow::Truncate, &mut cursor, &mut points)
            .unwrap();
        assert_eq!(
            rows(&g),
            ["abcd\\", "xyz", ""],
            "the cut wide character is blanked"
        );
        assert!(points[0].clamped);
        assert_eq!(
            (points[1], cursor),
            (TrackPoint::new(1, 2), TrackPoint::new(1, 3))
        );
        g.resize(9, 3, Reflow::Truncate, &mut cursor, &mut points)
            .unwrap();
        assert_eq!(rows(&g), ["abcd\\", "xyz", ""]);
        let after: Vec<_> = (0..3).map(|i| g.row(i).unwrap().id()).collect();
        assert_eq!(after, ids, "rows are kept, not rebuilt");
        assert_eq!(g.history_len(), 0);
    }

    #[test]
    fn zero_rows_or_columns_are_refused_and_change_nothing() {
        let mut g = Grid::new(4, 2, 0).unwrap();
        write(&mut g, "ab");
        let mut cursor = TrackPoint::new(0, 2);
        for (cols, screen) in [(0, 2), (4, 0)] {
            let r = g.resize(cols, screen, Reflow::Rewrap, &mut cursor, &mut []);
            assert_eq!(r, Err(GridError::Empty));
        }
        assert_eq!(rows(&g), ["ab", ""]);
        assert_eq!(cursor, TrackPoint::new(0, 2));
    }

    #[test]
    fn points_outside_the_grid_are_clamped_and_reported() {
        let mut g = Grid::new(4, 2, 0).unwrap();
        let mut cursor = TrackPoint::new(usize::MAX, u16::MAX);
        let mut points = [TrackPoint::new(0, 9), TrackPoint::new(1, 1)];
        resize(&mut g, 4, 2, &mut cursor, &mut points);
        assert_eq!((cursor.row, cursor.col, cursor.clamped), (1, 3, true));
        assert!(points[0].clamped && points[0].col == 3);
        assert!(!points[1].clamped);
    }

    #[test]
    fn same_size_changes_nothing() {
        let mut g = Grid::new(5, 2, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcdefg");
        let before: Vec<_> = (0..total(&g))
            .map(|i| (g.row(i).unwrap().id(), g.row(i).unwrap().generation()))
            .collect();
        let at = cursor;
        resize(&mut g, 5, 2, &mut cursor, &mut []);
        let after: Vec<_> = (0..total(&g))
            .map(|i| (g.row(i).unwrap().id(), g.row(i).unwrap().generation()))
            .collect();
        assert_eq!((after, cursor), (before, at));
    }

    #[test]
    fn rows_keep_their_id_where_they_start_and_others_get_fresh_ids() {
        let mut g = Grid::new(6, 3, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "ab\nabcdefgh");
        let id = |g: &Grid, i: usize| g.row(i).unwrap().id();
        let generation = |g: &Grid, i: usize| g.row(i).unwrap().generation();
        let (short, head, tail) = (id(&g, 0), id(&g, 1), id(&g, 2));
        let (short_gen, head_gen) = (generation(&g, 0), generation(&g, 1));
        resize(&mut g, 4, 3, &mut cursor, &mut []);
        assert_eq!(
            rows(&g),
            ["ab", "abcd\\", "efgh\\", ""],
            "the cursor past the text keeps a row"
        );
        assert_eq!(
            (id(&g, 0), generation(&g, 0)),
            (short, short_gen),
            "unchanged row"
        );
        assert_eq!(id(&g, 1), head, "a line's first row keeps its id");
        assert_ne!(generation(&g, 1), head_gen, "but its content changed");
        assert!(
            ![short, head, tail].contains(&id(&g, 2)),
            "a new row gets a fresh id"
        );
        let all: Vec<_> = (0..total(&g)).map(|i| id(&g, i)).collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn changing_rows_only_keeps_every_row() {
        let mut g = Grid::new(5, 4, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcdefg\nhi\njk\nl");
        let before = rows(&g);
        let ids: Vec<_> = (0..total(&g)).map(|i| g.row(i).unwrap().id()).collect();
        resize(&mut g, 5, 2, &mut cursor, &mut []);
        assert_eq!(rows(&g), before);
        assert_eq!(g.history_len(), 3);
        let after: Vec<_> = (0..total(&g)).map(|i| g.row(i).unwrap().id()).collect();
        assert_eq!(after, ids);
    }

    #[test]
    fn growing_rows_pulls_history_back_onto_the_screen() {
        let mut g = Grid::new(4, 2, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "a\nb\nc\nd");
        assert_eq!(g.history_len(), 2);
        resize(&mut g, 4, 4, &mut cursor, &mut []);
        assert_eq!(g.history_len(), 0);
        assert_eq!(cursor, TrackPoint::new(3, 1));
        assert_eq!(g.screen_row(0).unwrap().cell(0).map(Cell::content), ch('a'));
    }

    #[test]
    fn blank_rows_below_the_cursor_absorb_a_shrink_before_history_does() {
        let mut g = Grid::new(8, 4, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "abcdefgh");
        resize(&mut g, 4, 4, &mut cursor, &mut []);
        assert_eq!(g.history_len(), 0);
        assert_eq!(rows(&g), ["abcd\\", "efgh", "", ""]);
    }

    #[test]
    fn rows_below_the_cursor_that_do_not_fit_are_dropped() {
        let mut g = Grid::new(4, 4, BIG_HISTORY).unwrap();
        write(&mut g, "a\nb\nc\nd");
        let mut cursor = TrackPoint::new(0, 0);
        let mut points = [TrackPoint::new(3, 0)];
        resize(&mut g, 4, 2, &mut cursor, &mut points);
        assert_eq!(rows(&g), ["a", "b"]);
        assert_eq!(cursor, TrackPoint::new(0, 0));
        assert!(points[0].clamped && points[0].row == 1);
    }

    #[test]
    fn styles_and_links_travel_with_their_cells() {
        let mut g = Grid::new(6, 2, BIG_HISTORY).unwrap();
        let styled: Vec<_> = "abcdef"
            .chars()
            .map(|c| (c, if c > 'c' { RED } else { StyleId::DEFAULT }))
            .collect();
        let mut cursor = write_styled(&mut g, &styled);
        g.screen_row_mut(0).unwrap().set_link(2..5, Some(LinkId(7)));
        resize(&mut g, 4, 2, &mut cursor, &mut []);
        assert_eq!(rows(&g), ["abcd\\", "ef"]);
        let style = |r: usize, c: u16| g.row(r).unwrap().cell(c).unwrap().style();
        assert_eq!(
            (style(0, 2), style(0, 3), style(1, 1)),
            (StyleId::DEFAULT, RED, RED)
        );
        let link = |r: usize, c: u16| g.row(r).unwrap().link_at(c);
        assert_eq!(
            (link(0, 1), link(0, 2), link(0, 3)),
            (None, Some(LinkId(7)), Some(LinkId(7)))
        );
        assert_eq!((link(1, 0), link(1, 1)), (Some(LinkId(7)), None));
        resize(&mut g, 6, 2, &mut cursor, &mut []);
        let spans = g.row(0).unwrap().links().to_vec();
        assert_eq!(spans.len(), 1, "the halves join again");
        assert_eq!((spans[0].start, spans[0].end), (2, 5));
    }

    #[test]
    fn erased_background_survives_as_the_line_fill() {
        let mut g = Grid::new(6, 2, BIG_HISTORY).unwrap();
        g.screen_row_mut(0).unwrap().clear(Cell::blank(RED));
        let mut cursor = write(&mut g, "ab");
        resize(&mut g, 10, 2, &mut cursor, &mut []);
        let row = g.row(0).unwrap();
        assert_eq!(
            row.cell(9),
            Some(Cell::blank(RED)),
            "the erase colour pads the new width"
        );
    }

    #[test]
    fn one_column_grid_blanks_wide_characters() {
        let mut g = Grid::new(4, 2, BIG_HISTORY).unwrap();
        let mut cursor = write(&mut g, "a中b");
        let mut points = [TrackPoint::new(0, 1), TrackPoint::new(0, 2)];
        resize(&mut g, 1, 2, &mut cursor, &mut points);
        assert_eq!(lines(&g), ["a b"]);
        assert!(points.iter().all(|p| p.clamped));
        assert_eq!(cell_at(&g, cursor).map(Cell::content), ch('b'));
    }

    /// A random screenful: letters, wide characters and line breaks, each
    /// with one of a few styles.
    fn text() -> impl Strategy<Value = Vec<(char, StyleId)>> {
        let ch = prop_oneof![
            6 => prop::char::range('a', 'z'),
            2 => prop::char::range('\u{4e00}', '\u{4e20}'),
            1 => Just(' '),
            1 => Just('\n'),
        ];
        prop::collection::vec((ch, (0..3u16).prop_map(StyleId)), 0..120)
    }

    fn sizes() -> impl Strategy<Value = Vec<(u16, u16)>> {
        prop::collection::vec((2..14u16, 1..7u16), 1..6)
    }

    proptest! {
        #[test]
        fn rewrap_sequences_lose_no_point_and_no_line(
            start in (2..14u16, 1..7u16),
            text in text(),
            picks in prop::collection::vec((any::<prop::sample::Index>(), any::<u16>()), 0..6),
            sizes in sizes(),
        ) {
            let mut g = Grid::new(start.0, start.1, BIG_HISTORY).unwrap();
            let mut cursor = write_styled(&mut g, &text);
            let mut points: Vec<_> = picks
                .iter()
                .map(|(i, c)| TrackPoint::new(i.index(cursor.row + 1), c % g.cols()))
                // Rows below the cursor may not fit on a shorter screen, and a
                // placeholder cell vanishes by design when its line joins.
                .filter(|p| (p.row, p.col) <= (cursor.row, cursor.col))
                .filter(|p| !cell_at(&g, *p).is_some_and(|c| c.flags().contains(CellFlags::LEADING_SPACER)))
                .collect();
            let original_lines = lines(&g);
            let cursor_cell = cell_at(&g, cursor);
            let cells: Vec<_> = points.iter().map(|p| cell_at(&g, *p)).collect();
            for (cols, screen) in sizes {
                resize(&mut g, cols, screen, &mut cursor, &mut points);
                prop_assert!(!cursor.clamped && cursor.row >= g.history_len() && cursor.row < total(&g));
                prop_assert_eq!(cell_at(&g, cursor), cursor_cell);
                for (p, cell) in points.iter().zip(&cells) {
                    prop_assert!(!p.clamped, "{:?} lost", p);
                    prop_assert_eq!(cell_at(&g, *p), *cell);
                }
                prop_assert_eq!(lines(&g), original_lines.clone());
            }
        }

        #[test]
        fn any_resize_sequence_keeps_points_in_the_grid_and_the_ring_sound(
            start in (1..14u16, 1..7u16),
            history in 0..10usize,
            text in text(),
            raw in prop::collection::vec((any::<usize>(), any::<u16>()), 0..6),
            steps in prop::collection::vec((1..14u16, 1..7u16, any::<bool>(), any::<i8>()), 1..6),
        ) {
            let mut g = Grid::new(start.0, start.1, history).unwrap();
            let mut cursor = write_styled(&mut g, &text);
            let mut points: Vec<_> = raw.iter().map(|&(r, c)| TrackPoint::new(r % (total(&g) + 2), c)).collect();
            for (cols, screen, rewrap, scroll) in steps {
                g.scroll_display(isize::from(scroll));
                let cells: Vec<_> = points.iter().map(|p| cell_at(&g, *p)).collect();
                let mode = if rewrap { Reflow::Rewrap } else { Reflow::Truncate };
                g.resize(cols, screen, mode, &mut cursor, &mut points).unwrap();
                prop_assert!(cursor.row >= g.history_len() && cursor.row < total(&g));
                for (p, cell) in points.iter().zip(cells) {
                    prop_assert!(p.row < total(&g) && p.col < g.cols());
                    if !p.clamped {
                        prop_assert_eq!(cell_at(&g, *p), cell);
                    }
                }
                prop_assert!(g.history_len() <= g.history_limit());
                prop_assert!(g.display_offset() <= g.history_len());
                prop_assert!(g.rows.len() <= g.capacity());
                let mut ids: Vec<RowId> = (0..total(&g)).map(|i| g.row(i).unwrap().id()).collect();
                prop_assert!(ids.iter().all(|id| *id != RowId::default()));
                ids.sort_unstable();
                ids.dedup();
                prop_assert_eq!(ids.len(), total(&g));
                prop_assert!((0..total(&g)).all(|i| g.row(i).unwrap().cols() == g.cols()));
            }
        }
    }
}
