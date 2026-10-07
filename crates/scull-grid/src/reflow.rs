//! Rewrapping rows to a new width in one pass, carrying tracking points.
//! Separate from the ring so the line arithmetic is tested on plain row
//! lists; the grid decides afterwards which rows become screen and history.
//!
//! The approach follows foot (`docs/research/foot-contour.md` §1.7, ideas
//! only, nothing ported): rows joined by their soft-wrap flag form logical
//! lines, every line is cut again at the new width while the points, sorted
//! by position, are translated as the copy passes them. Cost is one visit
//! per carried cell plus sorting the points.

use crate::cell::{Cell, CellFlags};
use crate::row::{LinkId, LinkSpan, Row, RowId};

/// A position in a list of rows: row index and column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Spot {
    pub(crate) row: usize,
    pub(crate) col: u16,
    /// Whether the cell that was here may have changed or vanished.
    pub(crate) lost: bool,
}

/// Where one fed cell ended up.
#[derive(Clone, Copy)]
struct Landing {
    row: usize,
    col: u16,
    exact: bool,
}

/// The row being assembled and everything already finished.
struct Out {
    cols: u16,
    rows: Vec<Row>,
    cells: Vec<Cell>,
    links: Vec<LinkSpan>,
    /// The old row whose first cell starts the row being built: its id lives on.
    origin: Option<usize>,
    /// The old row whose first cell has not been placed yet.
    candidate: Option<usize>,
    /// A wide head was laid down and its spacer is due next.
    after_head: bool,
    /// A wide head was blanked (a one-column grid) and its spacer is dropped.
    head_dropped: bool,
    last: Landing,
    next_id: u64,
}

/// Rewraps `old` (oldest first) to `cols` columns and moves every point in
/// `spots` (old coordinates in, new ones out) with its cell. Old rows are
/// emptied as their line is consumed, so peak memory is one extra line.
pub(crate) fn rewrap(
    old: &mut [Row],
    cols: u16,
    spots: &mut [Spot],
    next_id: &mut u64,
) -> Vec<Row> {
    let mut keys: Vec<(usize, u16, usize)> = spots
        .iter()
        .enumerate()
        .map(|(i, s)| (s.row, s.col, i))
        .collect();
    keys.sort_unstable();
    let mut next = 0;
    let mut out = Out {
        cols,
        rows: Vec::with_capacity(old.len()),
        cells: Vec::new(),
        links: Vec::new(),
        origin: None,
        candidate: None,
        after_head: false,
        head_dropped: false,
        last: Landing {
            row: 0,
            col: 0,
            exact: false,
        },
        next_id: *next_id,
    };
    let mut start = 0;
    while start < old.len() {
        let end = line_end(old, start);
        let fill = line_fill(old.get(end));
        let mut fed = 0;
        for r in start..=end {
            let Some(row) = old.get(r) else { break };
            let pending = keys.get(next..).unwrap_or_default();
            let point_end = pending
                .iter()
                .take_while(|k| k.0 <= r)
                .filter(|k| k.0 == r)
                .map(|k| k.1.saturating_add(1))
                .max()
                .unwrap_or(0);
            let kept = kept_len(row, r == end, point_end);
            out.candidate = Some(r);
            let mut spans = row.links().iter().peekable();
            for (c, cell) in (0..kept).zip(row.cells()) {
                while spans.next_if(|s| s.end <= c).is_some() {}
                let link = spans.peek().filter(|s| s.start <= c).map(|s| s.link);
                let at = out.feed(old, cell, link, fill);
                fed += 1;
                while let Some(&(pr, pc, i)) = keys.get(next)
                    && (pr, pc) <= (r, c)
                {
                    let exact = at.exact && (pr, pc) == (r, c);
                    put(spots, i, at.row, at.col, exact);
                    next += 1;
                }
            }
        }
        let tail = out.end_line(old, start, fed, fill);
        // Points left in this line sat on a dropped leading spacer with
        // nothing after it; they stay at the line's end.
        while let Some(&(pr, _, i)) = keys.get(next)
            && pr <= end
        {
            put(spots, i, tail.row, tail.col, false);
            next += 1;
        }
        for row in old.get_mut(start..=end).unwrap_or_default() {
            *row = Row::new(RowId::default(), 0, Cell::EMPTY);
        }
        start = end + 1;
    }
    *next_id = out.next_id;
    out.rows
}

fn put(spots: &mut [Spot], i: usize, row: usize, col: u16, exact: bool) {
    if let Some(s) = spots.get_mut(i) {
        s.row = row;
        s.col = col;
        s.lost |= !exact;
    }
}

/// The last row of the logical line that starts at `start`.
fn line_end(old: &[Row], start: usize) -> usize {
    let mut end = start;
    while end + 1 < old.len() && old.get(end).is_some_and(Row::wrapped) {
        end += 1;
    }
    end
}

/// The fill every row of a line is padded with: the last row's when it is
/// erased space, else plain blank, since a text fill only spoke for the old
/// width.
fn line_fill(last: Option<&Row>) -> Cell {
    last.filter(|r| r.fill_is_blank())
        .map_or(Cell::EMPTY, Row::fill)
}

/// Cells of `row` that belong to its line. A wrapped row drops the
/// placeholder a wide character left at its edge; the last row stops after
/// its content, or after the furthest point on it so a cursor past the text
/// keeps its column.
fn kept_len(row: &Row, last: bool, point_end: u16) -> u16 {
    if last {
        return row.content_len().max(point_end).min(row.cols());
    }
    let edge = row.cols().saturating_sub(1);
    let leading = row
        .cell(edge)
        .is_some_and(|c| c.flags().contains(CellFlags::LEADING_SPACER));
    if leading { edge } else { row.cols() }
}

impl Out {
    /// Lays one cell of a line down and says where it went.
    fn feed(&mut self, old: &[Row], cell: Cell, link: Option<LinkId>, fill: Cell) -> Landing {
        let flags = cell.flags();
        if flags.contains(CellFlags::SPACER) {
            if std::mem::take(&mut self.head_dropped) {
                return Landing {
                    exact: false,
                    ..self.last
                };
            }
            if std::mem::take(&mut self.after_head) {
                return self.push(cell, link);
            }
        }
        self.after_head = false;
        let mut cell = cell;
        let mut width = 1;
        let mut exact = true;
        if flags.contains(CellFlags::WIDE) {
            if self.cols < 2 {
                // A wide character wider than the grid: nothing can show it.
                cell = Cell::blank(cell.style());
                self.head_dropped = true;
                exact = false;
            } else {
                width = 2;
                self.after_head = true;
            }
        }
        let used = self.cells.len();
        if used + width > usize::from(self.cols) {
            if width == 2 && used + 1 == usize::from(self.cols) {
                self.cells
                    .push(Cell::EMPTY.with_flags(CellFlags::LEADING_SPACER));
            }
            self.finish_row(old, true, fill);
        }
        let at = self.push(cell, link);
        Landing { exact, ..at }
    }

    fn push(&mut self, cell: Cell, link: Option<LinkId>) -> Landing {
        let candidate = self.candidate.take();
        if self.cells.is_empty() {
            self.origin = candidate;
            self.cells.reserve_exact(usize::from(self.cols));
        }
        let col = u16::try_from(self.cells.len()).unwrap_or(u16::MAX);
        self.cells.push(cell);
        if let Some(link) = link {
            match self.links.last_mut() {
                Some(span) if span.link == link && span.end == col => span.end = col + 1,
                _ => self.links.push(LinkSpan {
                    start: col,
                    end: col + 1,
                    link,
                }),
            }
        }
        self.last = Landing {
            row: self.rows.len(),
            col,
            exact: true,
        };
        self.last
    }

    /// Ends a line; an empty one still yields a row, keeping its first old
    /// row's id. Returns the spot just past the line's last cell.
    fn end_line(&mut self, old: &[Row], first: usize, fed: usize, fill: Cell) -> Landing {
        if fed == 0 {
            self.origin = Some(first);
        }
        self.candidate = None;
        self.after_head = false;
        self.head_dropped = false;
        let col = u16::try_from(self.cells.len())
            .unwrap_or(u16::MAX)
            .min(self.cols.saturating_sub(1));
        let row = self.rows.len();
        self.finish_row(old, false, fill);
        Landing {
            row,
            col,
            exact: false,
        }
    }

    fn finish_row(&mut self, old: &[Row], wrapped: bool, fill: Cell) {
        let origin = self.origin.take().and_then(|i| old.get(i));
        let (id, generation) = match origin {
            Some(o) => (o.id(), o.generation()),
            None => {
                self.next_id = self.next_id.wrapping_add(1);
                (RowId(self.next_id), 0)
            }
        };
        let cells = std::mem::take(&mut self.cells);
        let links = std::mem::take(&mut self.links);
        let mut row = Row::rebuilt(id, generation, self.cols, wrapped, fill, cells, links);
        if origin.is_some_and(|o| !row.same_content(o)) {
            row.mark_changed();
        }
        self.rows.push(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::StyleId;

    const COLS: u16 = 4;

    fn row(id: u64, text: &str, wrapped: bool) -> Row {
        let mut r = Row::new(RowId(id), COLS, Cell::EMPTY);
        for (col, c) in (0..).zip(text.chars()) {
            let cell = match c {
                '>' => Cell::EMPTY.with_flags(CellFlags::LEADING_SPACER),
                _ => Cell::char(c, StyleId::DEFAULT),
            };
            r.set(col, cell).unwrap();
        }
        r.set_wrapped(wrapped);
        r
    }

    #[test]
    fn only_a_wrapped_row_drops_its_edge_placeholder() {
        assert_eq!(kept_len(&row(1, "abc>", true), false, 0), 3);
        assert_eq!(kept_len(&row(1, "abc>", false), true, 0), 4);
        assert_eq!(kept_len(&row(1, "ab", false), true, 0), 2);
        assert_eq!(
            kept_len(&row(1, "ab", false), true, 3),
            3,
            "a point extends the line"
        );
    }

    #[test]
    fn a_row_whose_fill_is_text_keeps_every_column() {
        let mut r = Row::new(RowId(1), COLS, Cell::char('z', StyleId::DEFAULT));
        r.compact();
        assert_eq!(kept_len(&r, true, 0), COLS);
        assert_eq!(line_fill(Some(&r)), Cell::EMPTY);
    }

    #[test]
    fn consumed_rows_are_emptied_and_empty_lines_keep_their_id() {
        let mut old = vec![row(1, "abcd", true), row(2, "ef", false), row(3, "", false)];
        let mut next_id = 10;
        let rows = rewrap(&mut old, 3, &mut [], &mut next_id);
        assert!(old.iter().all(|r| r.cols() == 0), "old rows were released");
        let ids: Vec<_> = rows.iter().map(Row::id).collect();
        assert_eq!(ids, [RowId(1), RowId(11), RowId(3)]);
        assert_eq!(next_id, 11);
    }
}
