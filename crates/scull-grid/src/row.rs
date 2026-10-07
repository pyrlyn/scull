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
        self.drop_links(usize::from(start), usize::from(end));
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
        }
        self.touch();
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
            self.drop_links(cut, usize::from(self.cols));
            if lost {
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

    fn drop_links(&mut self, start: usize, end: usize) {
        let Some(extra) = &mut self.extra else {
            return;
        };
        let (Ok(start), Ok(end)) = (u16::try_from(start), u16::try_from(end)) else {
            return;
        };
        let mut kept = Vec::with_capacity(extra.links.len() + 1);
        for s in extra.links.drain(..) {
            if s.end <= start || s.start >= end {
                kept.push(s);
                continue;
            }
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
