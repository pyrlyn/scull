//! One row of cells with its identity, generation and out-of-line extras.
//! Separate from the ring so storage tricks (uniform rows, trimmed tails,
//! wide-character fix-ups) are tested without scrolling.
//!
//! Storage is a written prefix plus a `fill` cell standing for every column
//! past it, so a blank or uniformly erased row (Contour's "trivial line",
//! `docs/research/foot-contour.md` §2.4) has no cell allocation at all, and a
//! history row keeps only the columns up to its last non-fill cell.
//!
//! For damage (T9) a row carries a stable [`RowId`], fixed while the same
//! logical line scrolls through screen and history and replaced when the
//! slot is recycled, and a generation bumped by every visible change. A
//! frame that cached `(id, generation)` per row repaints exactly the rows
//! whose pair differs, and reads a scroll as known ids at new positions.

use std::ops::Range;

use crate::GridError;
use crate::cell::{Cell, CellFlags, Content};
use crate::intern::Marks;

/// Identity of a logical row, unique within one grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowId(pub u64);

/// An OSC 8 hyperlink as the terminal layer numbers it; the grid only keeps
/// which columns carry which id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinkId(pub u32);

/// Columns `start..end` of a row carry `link`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// First column.
    pub start: u16,
    /// One past the last column.
    pub end: u16,
    /// The link.
    pub link: LinkId,
}

/// Rare per-row data, boxed so rows without it pay one null pointer.
/// Spans are sorted and disjoint, so a row never holds more than `cols`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RowExtra {
    links: Vec<LinkSpan>,
}

/// One row of the grid.
#[derive(Clone, Debug)]
pub struct Row {
    id: RowId,
    generation: u32,
    cols: u16,
    wrapped: bool,
    fill: Cell,
    cells: Vec<Cell>,
    extra: Option<Box<RowExtra>>,
}

impl Row {
    /// A row of `cols` copies of `fill`, with no cell allocation.
    pub fn new(id: RowId, cols: u16, fill: Cell) -> Self {
        Self {
            id,
            generation: 0,
            cols,
            wrapped: false,
            fill,
            cells: Vec::new(),
            extra: None,
        }
    }

    /// Stable identity of the logical line in this row.
    pub fn id(&self) -> RowId {
        self.id
    }

    /// Bumped by every change a frame must repaint.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Width in columns.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Whether every column holds the same cell, without allocation.
    pub fn is_uniform(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether the line continues in the next row (soft wrap), for reflow
    /// and copy.
    pub fn wrapped(&self) -> bool {
        self.wrapped
    }

    /// Marks or clears the soft wrap into the next row.
    pub fn set_wrapped(&mut self, wrapped: bool) {
        if self.wrapped != wrapped {
            self.wrapped = wrapped;
            self.touch();
        }
    }

    /// The cell at `col`, `None` past the row.
    pub fn cell(&self, col: u16) -> Option<Cell> {
        if col >= self.cols {
            return None;
        }
        Some(
            self.cells
                .get(usize::from(col))
                .copied()
                .unwrap_or(self.fill),
        )
    }

    /// Every column, left to right.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        let tail = usize::from(self.cols).saturating_sub(self.cells.len());
        self.cells
            .iter()
            .copied()
            .chain(std::iter::repeat_n(self.fill, tail))
    }

    /// Stores `cell` at `col` as is. No wide-character fix-up: for cells the
    /// caller lays out itself, such as a [`CellFlags::LEADING_SPACER`].
    pub fn set(&mut self, col: u16, cell: Cell) -> Result<(), GridError> {
        let at = self.check(col)?;
        if self.cell(col) != Some(cell) {
            if let Some(slot) = self.materialize(at + 1).get_mut(at) {
                *slot = cell;
            }
            self.touch();
        }
        Ok(())
    }

    /// Writes a character at `col`; a `wide` one also writes the spacer to
    /// its right. A wide character half overwritten here loses its other half,
    /// so no head is left without a spacer or spacer without a head.
    pub fn put(&mut self, col: u16, cell: Cell, wide: bool) -> Result<(), GridError> {
        let start = self.check(col)?;
        let end = start + if wide { 2 } else { 1 };
        if end > usize::from(self.cols) {
            return Err(GridError::Column {
                col: col.saturating_add(1),
                cols: self.cols,
            });
        }
        self.break_wide_edges(start, end);
        let cells = self.materialize(end);
        if let Some(head) = cells.get_mut(start) {
            *head = if wide {
                cell.with_flags(CellFlags::WIDE)
            } else {
                cell.without_flags(CellFlags::WIDE)
            };
        }
        if wide && let Some(spacer) = cells.get_mut(start + 1) {
            *spacer = Cell::spacer(cell.style());
        }
        self.touch();
        Ok(())
    }

    /// Sets the columns in `range` to `cell` (EL, ECH, DCH tails) and drops
    /// their links. Covering the whole row makes it uniform again.
    pub fn fill_range(&mut self, range: Range<u16>, cell: Cell) {
        let end = usize::from(range.end.min(self.cols));
        let start = usize::from(range.start).min(end);
        if start == end {
            return;
        }
        if start == 0 && end == usize::from(self.cols) {
            self.clear(cell);
            return;
        }
        self.break_wide_edges(start, end);
        if cell == self.fill && end >= self.cells.len() {
            self.cells.truncate(start);
        } else if let Some(span) = self.materialize(end).get_mut(start..end) {
            span.fill(cell);
        }
        self.drop_links(start, end);
        self.touch();
    }

    /// ICH: moves columns `range` right by `n` from its start; cells pushed
    /// past its end are dropped and `n` copies of `fill` open the gap.
    pub fn insert_cells(&mut self, range: Range<u16>, n: u16, fill: Cell) {
        self.shift(range, n, fill, true);
    }

    /// DCH: deletes `n` columns at the start of `range`; the rest of the
    /// range moves left and `n` copies of `fill` close it at its end.
    pub fn delete_cells(&mut self, range: Range<u16>, n: u16, fill: Cell) {
        self.shift(range, n, fill, false);
    }

    /// Copies `cells` into the columns from `at`, dropping what runs past
    /// the row (scrolling inside left and right margins). A wide character
    /// cut by either edge of the copy is blanked, so no half survives.
    pub fn write_cells(&mut self, at: u16, cells: &[Cell]) {
        let start = usize::from(at.min(self.cols));
        let end = (start + cells.len()).min(usize::from(self.cols));
        if start == end {
            return;
        }
        if let (Some(span), Some(src)) = (
            self.materialize(end).get_mut(start..end),
            cells.get(..end - start),
        ) {
            span.copy_from_slice(src);
        }
        self.repair_wide(start.saturating_sub(1), end + 1);
        self.drop_links(start, end);
        self.touch();
    }

    fn shift(&mut self, range: Range<u16>, n: u16, fill: Cell, right: bool) {
        let end = usize::from(range.end.min(self.cols));
        let start = usize::from(range.start).min(end);
        let n = usize::from(n).min(end - start);
        if n == 0 {
            return;
        }
        let cells = self.materialize(end);
        if let Some(span) = cells.get_mut(start..end) {
            let gap = if right {
                span.rotate_right(n);
                0..n
            } else {
                span.rotate_left(n);
                span.len() - n..span.len()
            };
            if let Some(gap) = span.get_mut(gap) {
                gap.fill(fill);
            }
        }
        self.repair_wide(start.saturating_sub(1), end + 1);
        self.drop_links(start, end);
        self.touch();
    }

    /// Blanks every half of a wide character in `start..end` whose other
    /// half a shift moved away or overwrote.
    fn repair_wide(&mut self, start: usize, end: usize) {
        for i in start..end.min(self.cells.len()) {
            let cell = self.cells.get(i).copied().unwrap_or(self.fill);
            let next = self.cells.get(i + 1).copied().unwrap_or(self.fill);
            let prev_wide = i
                .checked_sub(1)
                .and_then(|p| self.cells.get(p))
                .is_some_and(|c| c.flags().contains(CellFlags::WIDE));
            let orphan_head =
                cell.flags().contains(CellFlags::WIDE) && !next.flags().contains(CellFlags::SPACER);
            let orphan_spacer = cell.flags().contains(CellFlags::SPACER) && !prev_wide;
            if (orphan_head || orphan_spacer)
                && let Some(slot) = self.cells.get_mut(i)
            {
                *slot = Cell::blank(cell.style());
            }
        }
    }

    /// Every column becomes `cell` in O(1); links go too. The allocation is
    /// kept, since a screen row is usually rewritten right away.
    pub fn clear(&mut self, cell: Cell) {
        self.fill = cell;
        self.cells.clear();
        self.extra = None;
        self.touch();
    }

    /// The link at `col`.
    pub fn link_at(&self, col: u16) -> Option<LinkId> {
        let spans = &self.extra.as_ref()?.links;
        spans
            .iter()
            .find(|s| s.start <= col && col < s.end)
            .map(|s| s.link)
    }

    /// The link spans, sorted by column.
    pub fn links(&self) -> &[LinkSpan] {
        self.extra.as_ref().map_or(&[], |e| &e.links)
    }

    /// Columns `range` carry `link` from now on (`None` removes links there).
    pub fn set_link(&mut self, range: Range<u16>, link: Option<LinkId>) {
        let end = range.end.min(self.cols);
        let start = range.start.min(end);
        if start == end {
            return;
        }
        // Clearing a range that holds no links changes nothing: without
        // this, every plain cell write that calls `set_link(.., None)`
        // would bump the generation and force a repaint (T22).
        let dropped = self.drop_links(usize::from(start), usize::from(end));
        if let Some(link) = link {
            let spans = &mut self.extra.get_or_insert_default().links;
            let at = spans.partition_point(|s| s.start < start);
            spans.insert(at, LinkSpan { start, end, link });
            spans.dedup_by(|next, prev| {
                let joins = prev.end == next.start && prev.link == next.link;
                if joins {
                    prev.end = next.end;
                }
                joins
            });
            self.touch();
        } else if dropped {
            self.touch();
        }
    }

    /// Heap and inline bytes this row holds: what one scrollback row costs.
    pub fn memory_bytes(&self) -> usize {
        size_of::<Self>()
            + self.cells.capacity() * size_of::<Cell>()
            + self.extra.as_ref().map_or(0, |e| {
                size_of::<RowExtra>() + e.links.capacity() * size_of::<LinkSpan>()
            })
    }

    /// Starts a new logical line in this slot: new id, no content.
    pub fn recycle(&mut self, id: RowId, fill: Cell) {
        self.id = id;
        self.generation = 0;
        self.wrapped = false;
        self.fill = fill;
        self.cells.clear();
        self.extra = None;
    }

    /// Shrinks the storage to what the content needs, for a row leaving the
    /// screen. Content and generation are unchanged.
    pub fn compact(&mut self) {
        let keep = self.stored_len();
        self.cells.truncate(keep);
        if let Some(&first) = self.cells.first()
            && self.cells.len() == usize::from(self.cols)
            && self.cells.iter().all(|c| *c == first)
        {
            self.fill = first;
            self.cells.clear();
        }
        self.cells.shrink_to_fit();
        if let Some(extra) = &mut self.extra {
            extra.links.shrink_to_fit();
        }
    }

    /// Marks every style and cluster this row refers to, for a sweep.
    pub fn mark(&self, styles: &mut Marks, clusters: &mut Marks) {
        for cell in self.cells.iter().chain(std::iter::once(&self.fill)) {
            styles.mark(u32::from(cell.style().0));
            if let Some(id) = cell.cluster_id() {
                clusters.mark(id.0);
            }
        }
    }

    /// A row assembled by reflow from cells and links it already laid out.
    pub(crate) fn rebuilt(
        id: RowId,
        generation: u32,
        cols: u16,
        wrapped: bool,
        fill: Cell,
        cells: Vec<Cell>,
        links: Vec<LinkSpan>,
    ) -> Self {
        let extra = (!links.is_empty()).then(|| Box::new(RowExtra { links }));
        let mut row = Self {
            id,
            generation,
            cols,
            wrapped,
            fill,
            cells,
            extra,
        };
        row.compact();
        row
    }

    /// The cell standing for every column past the stored prefix.
    pub(crate) fn fill(&self) -> Cell {
        self.fill
    }

    /// Whether the fill is erased space rather than text. Only then may the
    /// columns it stands for be dropped and re-padded at another width; a
    /// row compacted into a uniform run of `z` must keep its `z`s.
    pub(crate) fn fill_is_blank(&self) -> bool {
        self.fill.content() == Content::Empty && self.fill.flags().is_empty()
    }

    /// Columns a reflow must carry: up to the last cell that differs from a
    /// blank fill, or the whole row when the fill itself shows text.
    pub(crate) fn content_len(&self) -> u16 {
        if !self.fill_is_blank() {
            return self.cols;
        }
        u16::try_from(self.stored_len()).unwrap_or(self.cols)
    }

    /// Never written since it was blanked: reflow may drop it from the
    /// bottom of the screen without losing anything a frame shows.
    pub(crate) fn is_blank(&self) -> bool {
        self.stored_len() == 0 && self.fill == Cell::EMPTY && !self.wrapped && self.extra.is_none()
    }

    /// Whether `other` shows the same cells, links and wrap, whatever the
    /// widths: a reflowed row that passes keeps its generation.
    pub(crate) fn same_content(&self, other: &Self) -> bool {
        self.wrapped == other.wrapped
            && self.fill == other.fill
            && self.links() == other.links()
            && self.cells.get(..self.stored_len()) == other.cells.get(..other.stored_len())
    }

    /// Bumps the generation for a change made outside the cell setters.
    pub(crate) fn mark_changed(&mut self) {
        self.touch();
    }

    /// Changes the width without rewrapping (the alt screen): a narrower row
    /// loses its tail and a wide character cut in half, a wider one pads.
    pub(crate) fn set_cols(&mut self, cols: u16) {
        let cut = usize::from(cols);
        if cols > self.cols {
            if !self.fill_is_blank() {
                // The fill shows text only up to the old edge; new columns are blank.
                self.materialize(usize::from(self.cols));
                self.fill = Cell::EMPTY;
            }
        } else if cols < self.cols {
            let lost = self.stored_len() > cut || !self.fill_is_blank();
            self.break_wide_edges(cut, cut);
            self.cells.truncate(cut);
            // Links dropped off the tail are a visible change even when no
            // stored cell was lost (T22).
            let dropped = self.drop_links(cut, usize::from(self.cols));
            if lost || dropped {
                self.touch();
            }
        }
        self.cols = cols;
    }

    /// Stored cells up to the last one that is not the fill.
    fn stored_len(&self) -> usize {
        self.cells
            .iter()
            .rposition(|c| *c != self.fill)
            .map_or(0, |i| i + 1)
    }

    fn check(&self, col: u16) -> Result<usize, GridError> {
        if col < self.cols {
            Ok(usize::from(col))
        } else {
            Err(GridError::Column {
                col,
                cols: self.cols,
            })
        }
    }

    fn touch(&mut self) {
        // Wraps at `u32::MAX`: a frame cache keyed on `(RowId, generation)`
        // can miss one change after exactly 2^32 bumps of a single row —
        // accepted; T9's damage tracking must not rely on more.
        self.generation = self.generation.wrapping_add(1);
    }

    /// Ensures the first `len` columns are stored and returns them. The first
    /// write reserves the whole row so typing a line reallocates once.
    fn materialize(&mut self, len: usize) -> &mut [Cell] {
        if self.cells.len() < len {
            let cols = usize::from(self.cols);
            self.cells
                .reserve_exact(cols.saturating_sub(self.cells.len()));
            self.cells.resize(len.min(cols), self.fill);
        }
        &mut self.cells
    }

    /// Blanks the outer half of a wide character cut by `start..end`.
    fn break_wide_edges(&mut self, start: usize, end: usize) {
        let first = self.cells.get(start).copied();
        if first.is_some_and(|c| c.flags().contains(CellFlags::SPACER))
            && let Some(head) = start.checked_sub(1).and_then(|i| self.cells.get_mut(i))
        {
            *head = Cell::blank(head.style());
        }
        let last = end.checked_sub(1).and_then(|i| self.cells.get(i)).copied();
        if last.is_some_and(|c| c.flags().contains(CellFlags::WIDE))
            && let Some(spacer) = self.cells.get_mut(end)
        {
            *spacer = Cell::blank(spacer.style());
        }
    }

    fn drop_links(&mut self, start: usize, end: usize) -> bool {
        let Some(extra) = &mut self.extra else {
            return false;
        };
        let (Ok(start), Ok(end)) = (u16::try_from(start), u16::try_from(end)) else {
            return false;
        };
        let mut kept = Vec::with_capacity(extra.links.len() + 1);
        let mut dropped = false;
        for s in extra.links.drain(..) {
            if s.end <= start || s.start >= end {
                kept.push(s);
                continue;
            }
            dropped = true;
            if s.start < start {
                kept.push(LinkSpan { end: start, ..s });
            }
            if s.end > end {
                kept.push(LinkSpan { start: end, ..s });
            }
        }
        if kept.is_empty() {
            self.extra = None;
        } else {
            extra.links = kept;
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Content;
    use crate::style::StyleId;

    const COLS: u16 = 10;
    const RED: StyleId = StyleId(1);

    fn row() -> Row {
        Row::new(RowId(1), COLS, Cell::EMPTY)
    }

    fn ch(c: char) -> Cell {
        Cell::char(c, StyleId::DEFAULT)
    }

    fn text(r: &Row) -> String {
        r.cells()
            .map(|c| match c.content() {
                Content::Char(c) => c,
                Content::Empty if c.flags().contains(CellFlags::SPACER) => '_',
                _ => '.',
            })
            .collect()
    }

    #[test]
    fn blank_row_allocates_no_cells() {
        let r = row();
        assert!(r.is_uniform());
        assert_eq!(r.memory_bytes(), size_of::<Row>());
        assert_eq!(r.cells().count(), usize::from(COLS));
    }

    #[test]
    fn erase_with_a_style_keeps_the_row_uniform() {
        let mut r = row();
        r.put(3, ch('x'), false).unwrap();
        r.fill_range(0..COLS, Cell::blank(RED));
        assert!(r.is_uniform());
        assert!(r.cells().all(|c| c == Cell::blank(RED)));
    }

    #[test]
    fn writing_past_the_row_is_an_error() {
        let mut r = row();
        let err = Err(GridError::Column {
            col: COLS,
            cols: COLS,
        });
        assert_eq!(r.put(COLS, ch('x'), false), err);
        assert_eq!(r.set(COLS, ch('x')), err);
        assert_eq!(r.put(COLS - 1, ch('x'), true), err);
        assert_eq!(r.cell(COLS), None);
    }

    #[test]
    fn every_visible_change_bumps_the_generation_and_a_no_op_does_not() {
        let mut r = row();
        let g0 = r.generation();
        r.set(0, Cell::EMPTY).unwrap();
        assert_eq!(r.generation(), g0, "same cell is not a change");
        r.put(0, ch('a'), false).unwrap();
        let g1 = r.generation();
        assert_ne!(g1, g0);
        r.set_link(0..1, Some(LinkId(1)));
        assert_ne!(r.generation(), g1);
        assert_eq!(r.id(), RowId(1), "edits keep the id");
    }

    /// T22: clearing a range that holds no links is not a visible change —
    /// every plain cell write clears links, and a bump per write would
    /// force a repaint per keystroke once T9 keys damage on generations.
    #[test]
    fn clearing_links_where_none_exist_is_generation_neutral() {
        let mut r = row();
        let g0 = r.generation();
        r.set_link(0..5, None);
        assert_eq!(r.generation(), g0, "nothing was linked");

        r.set_link(0..5, Some(LinkId(1)));
        let g1 = r.generation();
        assert_ne!(g1, g0, "a link was added");

        r.set_link(2..9, None);
        assert_ne!(r.generation(), g1, "the link was cut short");
    }

    /// T22: narrowing a row that loses only links (no stored cells, blank
    /// fill) is a visible change — the link is gone from the frame.
    #[test]
    fn narrowing_away_links_bumps_the_generation() {
        let mut r = row();
        r.set_link(8..COLS, Some(LinkId(1)));
        let g0 = r.generation();
        r.set_cols(COLS - 2);
        assert!(r.link_at(COLS - 2).is_none(), "the link is gone");
        assert_ne!(r.generation(), g0, "a link was lost off the tail");
    }

    #[test]
    fn wide_char_writes_head_and_spacer() {
        let mut r = row();
        r.put(2, ch('W'), true).unwrap();
        assert!(r.cell(2).unwrap().flags().contains(CellFlags::WIDE));
        assert_eq!(r.cell(3), Some(Cell::spacer(StyleId::DEFAULT)));
        assert_eq!(text(&r), "..W_......");
    }

    #[test]
    fn overwriting_half_a_wide_char_blanks_the_other_half() {
        let mut r = row();
        r.put(2, ch('W'), true).unwrap();
        r.put(3, ch('x'), false).unwrap();
        assert_eq!(text(&r), "...x......", "spacer overwritten: head blanked");
        r.put(5, ch('V'), true).unwrap();
        r.put(5, ch('y'), false).unwrap();
        assert_eq!(text(&r), "...x.y....", "head overwritten: spacer blanked");
        r.put(7, ch('A'), true).unwrap();
        r.put(6, ch('B'), true).unwrap();
        assert_eq!(text(&r), "...x.yB_..", "wide over a head blanks its spacer");
    }

    #[test]
    fn erasing_half_a_wide_char_blanks_the_other_half() {
        let mut r = row();
        r.put(2, ch('W'), true).unwrap();
        r.fill_range(3..4, Cell::EMPTY);
        assert_eq!(text(&r), "..........");
    }

    fn typed(s: &str) -> Row {
        let mut r = row();
        for (col, c) in (0..).zip(s.chars()) {
            if c != '.' {
                r.put(col, ch(c), false).unwrap();
            }
        }
        r
    }

    #[test]
    fn insert_cells_shifts_right_inside_the_range_only() {
        let mut r = typed("abcdefghij");
        r.insert_cells(2..8, 2, Cell::EMPTY);
        assert_eq!(text(&r), "ab..cdefij");
        r.insert_cells(0..COLS, COLS + 5, Cell::EMPTY);
        assert_eq!(text(&r), "..........", "a count past the range clears it");
    }

    #[test]
    fn delete_cells_shifts_left_and_fills_the_range_end() {
        let mut r = typed("abcdefghij");
        r.delete_cells(2..8, 2, Cell::blank(RED));
        assert_eq!(text(&r), "abefgh..ij");
        assert_eq!(r.cell(6), Some(Cell::blank(RED)));
    }

    #[test]
    fn shifting_never_leaves_half_a_wide_char() {
        let mut r = row();
        r.put(6, ch('W'), true).unwrap();
        r.insert_cells(0..8, 1, Cell::EMPTY);
        assert_eq!(text(&r), "..........", "spacer pushed past the range end");
        let mut r = row();
        r.put(2, ch('W'), true).unwrap();
        r.delete_cells(3..COLS, 1, Cell::EMPTY);
        assert_eq!(text(&r), "..........", "spacer deleted under the head");
        let mut r = row();
        r.put(4, ch('W'), true).unwrap();
        r.insert_cells(5..COLS, 1, Cell::EMPTY);
        assert_eq!(text(&r), "..........", "insert between head and spacer");
        let mut r = row();
        r.put(4, ch('W'), true).unwrap();
        r.insert_cells(0..COLS, 2, Cell::EMPTY);
        assert_eq!(text(&r), "......W_..", "a whole wide char moves intact");
    }

    #[test]
    fn write_cells_copies_a_span_and_clips_at_the_row_end() {
        let mut r = typed("abcdefghij");
        let src: Vec<Cell> = typed("xyz").cells().take(3).collect();
        r.write_cells(2, &src);
        assert_eq!(text(&r), "abxyzfghij");
        r.write_cells(8, &src);
        assert_eq!(text(&r), "abxyzfghxy", "the part past the row is dropped");
        r.write_cells(COLS, &src);
        assert_eq!(
            text(&r),
            "abxyzfghxy",
            "a copy starting past the row is a no-op"
        );
    }

    #[test]
    fn write_cells_never_leaves_half_a_wide_char() {
        let mut wide = row();
        wide.put(0, ch('W'), true).unwrap();
        let head_only: Vec<Cell> = wide.cells().take(1).collect();
        let spacer_only: Vec<Cell> = wide.cells().skip(1).take(1).collect();
        let mut r = typed("abcdefghij");
        r.write_cells(3, &head_only);
        assert_eq!(
            text(&r),
            "abc.efghij",
            "a head without its spacer is blanked"
        );
        r.write_cells(5, &spacer_only);
        assert_eq!(
            text(&r),
            "abc.e.ghij",
            "a spacer without its head is blanked"
        );
        let mut r = row();
        r.put(4, ch('V'), true).unwrap();
        r.write_cells(5, &[ch('q')]);
        assert_eq!(
            text(&r),
            ".....q....",
            "overwriting a spacer blanks its head"
        );
    }

    #[test]
    fn compact_trims_the_tail_and_collapses_uniform_rows() {
        let mut r = row();
        r.put(0, ch('a'), false).unwrap();
        r.put(1, ch('b'), false).unwrap();
        assert!(r.cells.capacity() >= usize::from(COLS));
        let g = r.generation();
        r.compact();
        assert_eq!(r.cells.capacity(), 2);
        assert_eq!(text(&r), "ab........");
        assert_eq!(r.generation(), g, "compaction is not a visible change");

        let mut full = row();
        for col in 0..COLS {
            full.put(col, ch('z'), false).unwrap();
        }
        full.compact();
        assert!(full.is_uniform());
        assert_eq!(text(&full), "zzzzzzzzzz");
    }

    #[test]
    fn link_spans_split_merge_and_stay_disjoint() {
        let mut r = row();
        r.set_link(0..6, Some(LinkId(1)));
        r.set_link(2..4, Some(LinkId(2)));
        assert_eq!(r.link_at(1), Some(LinkId(1)));
        assert_eq!(r.link_at(3), Some(LinkId(2)));
        assert_eq!(r.link_at(5), Some(LinkId(1)));
        assert_eq!(r.links().len(), 3);
        r.set_link(2..4, Some(LinkId(1)));
        assert_eq!(
            r.links(),
            &[LinkSpan {
                start: 0,
                end: 6,
                link: LinkId(1)
            }]
        );
        r.fill_range(0..3, Cell::EMPTY);
        assert_eq!(
            r.links(),
            &[LinkSpan {
                start: 3,
                end: 6,
                link: LinkId(1)
            }]
        );
        r.set_link(0..COLS, None);
        assert!(r.extra.is_none(), "no links, no side table");
    }

    #[test]
    fn link_spans_never_outnumber_columns() {
        let mut r = row();
        let mut state: u32 = 0x5eed;
        for _ in 0..1000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [a, b, id, _] = state.to_le_bytes();
            let (a, b) = (u16::from(a) % (COLS + 2), u16::from(b) % (COLS + 2));
            r.set_link(a.min(b)..a.max(b), Some(LinkId(u32::from(id % 3))));
            assert!(r.links().len() <= usize::from(COLS));
            assert!(r.links().windows(2).all(|w| w[0].end <= w[1].start));
            assert!(r.links().iter().all(|s| s.start < s.end && s.end <= COLS));
        }
    }

    #[test]
    fn mark_reaches_fill_and_cluster_cells() {
        use crate::cluster::ClusterId;
        let mut r = Row::new(RowId(1), COLS, Cell::blank(RED));
        r.put(0, Cell::cluster(ClusterId(4), StyleId(2)), false)
            .unwrap();
        let mut styles = Marks::new(3);
        let mut clusters = Marks::new(5);
        r.mark(&mut styles, &mut clusters);
        assert!(styles.is_marked(1) && styles.is_marked(2));
        assert!(clusters.is_marked(4));
    }
}
