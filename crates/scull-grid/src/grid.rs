//! The ring of rows shared by the screen and its scrollback, and the owner of
//! the style and cluster tables those rows refer to. Separate from the row so
//! the offset arithmetic and the sweep policy are tested on their own.
//!
//! Layout follows foot (`docs/research/foot-contour.md` §1.4-1.5) and
//! Alacritty's `Storage` (`docs/research/wezterm-alacritty.md` §1.5): one
//! power-of-two `Vec<Row>`, physical slot `(zero + logical) & mask`. Logical
//! row 0 is the oldest history row; the screen is the last `screen_rows`.
//! Slots are pushed only when first needed, so configured scrollback costs
//! nothing until output fills it. Scrolling the full screen moves `zero` (or
//! grows `history`) and recycles one row; no row is copied.
//!
//! The history limit is exactly what was asked for (up to [`MAX_RING_ROWS`]),
//! not rounded up to the ring: when the ring is larger than screen plus
//! history, the evicted oldest slot is emptied and left behind as a header
//! with no cells, so the extra slots cost `size_of::<Row>()` each at most.

use std::ops::Range;

use crate::cell::Cell;
use crate::cluster::{ClusterId, ClusterTable};
use crate::row::{Row, RowId};
use crate::style::{Style, StyleId, StyleTable};
use crate::{GridError, Marks};

mod resize;
pub use resize::{Reflow, TrackPoint};

/// Screen plus scrollback rows one grid may hold. Scrollback is configured,
/// not sent by the PTY, but a typo must not reserve gigabytes: at this cap
/// a ring of blank rows is a few MiB of row headers.
pub const MAX_RING_ROWS: usize = 1 << 17;

/// A sweep walks every row, so it runs only once it can pay for itself:
/// after `cap / SWEEP_DIVISOR` inserts into the full table, or after
/// `rows / SWEEP_DIVISOR` rows were recycled since the last one. Either way
/// its cost is a constant factor of work already done.
const SWEEP_DIVISOR: usize = 8;

/// The screen and its scrollback in one ring.
#[derive(Debug, Clone)]
pub struct Grid {
    rows: Vec<Row>,
    mask: usize,
    zero: usize,
    history: usize,
    history_limit: usize,
    /// The scrollback asked for, so a resize can recompute the clamped limit.
    scrollback: usize,
    screen_rows: u16,
    cols: u16,
    display_offset: usize,
    next_id: u64,
    styles: StyleTable,
    clusters: ClusterTable,
    recent_style: StyleId,
    recent_cluster: Option<ClusterId>,
    released_since_sweep: usize,
    epoch: u64,
}

impl Grid {
    /// A blank `cols` x `screen_rows` screen keeping up to `scrollback`
    /// history rows (clamped so screen plus history fit [`MAX_RING_ROWS`]).
    pub fn new(cols: u16, screen_rows: u16, scrollback: usize) -> Result<Self, GridError> {
        if cols == 0 || screen_rows == 0 {
            return Err(GridError::Empty);
        }
        let rows = usize::from(screen_rows);
        let history_limit = scrollback.min(MAX_RING_ROWS.saturating_sub(rows));
        let mask = (rows + history_limit).next_power_of_two() - 1;
        let mut grid = Self {
            rows: Vec::with_capacity(rows),
            mask,
            zero: 0,
            history: 0,
            history_limit,
            scrollback,
            screen_rows,
            cols,
            display_offset: 0,
            next_id: 0,
            styles: StyleTable::default(),
            clusters: ClusterTable::default(),
            recent_style: StyleId::DEFAULT,
            recent_cluster: None,
            released_since_sweep: 0,
            epoch: 0,
        };
        for _ in 0..rows {
            let id = grid.fresh_id();
            grid.rows.push(Row::new(id, cols, Cell::EMPTY));
        }
        Ok(grid)
    }

    /// The same grid with other interning tables, e.g. smaller caps.
    #[must_use]
    pub fn with_tables(self, styles: StyleTable, clusters: ClusterTable) -> Self {
        Self {
            styles,
            clusters,
            ..self
        }
    }

    /// Columns.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Screen rows.
    pub fn screen_rows(&self) -> u16 {
        self.screen_rows
    }

    /// History rows held now.
    pub fn history_len(&self) -> usize {
        self.history
    }

    /// History rows kept at most.
    pub fn history_limit(&self) -> usize {
        self.history_limit
    }

    /// Ring slots: a power of two covering screen plus history limit.
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// How many rows the viewport is scrolled back into history.
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    /// Logical row `i`: 0 is the oldest history row, `history_len()` the
    /// top of the screen.
    pub fn row(&self, i: usize) -> Option<&Row> {
        if i >= self.history + usize::from(self.screen_rows) {
            return None;
        }
        self.rows.get(self.phys(i))
    }

    /// Screen row `r`, ignoring the viewport.
    pub fn screen_row(&self, r: u16) -> Option<&Row> {
        self.row(self.history + usize::from(r))
            .filter(|_| r < self.screen_rows)
    }

    /// Screen row `r` for writing.
    pub fn screen_row_mut(&mut self, r: u16) -> Option<&mut Row> {
        if r >= self.screen_rows {
            return None;
        }
        let at = self.phys(self.history + usize::from(r));
        self.rows.get_mut(at)
    }

    /// Row `r` of the viewport, which may be scrolled back into history.
    pub fn visible_row(&self, r: u16) -> Option<&Row> {
        self.row(self.history.saturating_sub(self.display_offset) + usize::from(r))
            .filter(|_| r < self.screen_rows)
    }

    /// Moves the viewport `delta` rows back into history (negative: towards
    /// the screen), clamped to what history holds.
    pub fn scroll_display(&mut self, delta: isize) {
        self.display_offset = self
            .display_offset
            .saturating_add_signed(delta)
            .min(self.history);
    }

    /// Scrolls the whole screen up `n` rows: the top rows enter history
    /// (the oldest history rows leave once the limit is reached) and `n`
    /// rows of `fill` appear at the bottom. Costs one offset change and one
    /// recycled row per line.
    pub fn scroll_up(&mut self, n: u16, fill: Cell) {
        for _ in 0..n.min(self.screen_rows) {
            self.advance(fill);
        }
    }

    /// Scrolls screen rows `region` up `n` rows (DECSTBM). The full screen
    /// feeds history; a partial region rotates in place and drops its top.
    pub fn scroll_region_up(&mut self, region: Range<u16>, n: u16, fill: Cell) {
        let (start, end) = self.clamp_region(region);
        if start == 0 && end == usize::from(self.screen_rows) {
            return self.scroll_up(n, fill);
        }
        let n = usize::from(n).min(end - start);
        for i in start..end - n {
            self.swap_screen(i, i + n);
        }
        self.recycle_screen(end - n..end, fill);
    }

    /// Scrolls screen rows `region` down `n` rows (RI, SD). History is never
    /// touched; rows pushed off the bottom are dropped.
    pub fn scroll_region_down(&mut self, region: Range<u16>, n: u16, fill: Cell) {
        let (start, end) = self.clamp_region(region);
        let n = usize::from(n).min(end - start);
        for i in (start + n..end).rev() {
            self.swap_screen(i, i - n);
        }
        self.recycle_screen(start..start + n, fill);
    }

    /// Drops every history row (ED 3); the screen is unchanged.
    pub fn clear_history(&mut self) {
        for i in 0..self.history {
            let at = self.phys(i);
            if let Some(row) = self.rows.get_mut(at) {
                *row = Row::new(RowId::default(), self.cols, Cell::EMPTY);
            }
        }
        self.released_since_sweep += self.history;
        self.zero = self.phys(self.history);
        self.history = 0;
        self.display_offset = 0;
    }

    /// The id of `style`. When the table is full, unused styles are swept
    /// first if that pays off: after an eighth of the cap in inserts, or an
    /// eighth of the ring in recycled rows, since the last sweep.
    pub fn intern_style(&mut self, style: &Style) -> Result<StyleId, GridError> {
        let id = match self.styles.intern(style) {
            Err(GridError::StyleTableFull { .. })
                if self.sweep_pays(self.styles.inserts_since_sweep(), self.styles.cap()) =>
            {
                self.sweep();
                self.styles.intern(style)
            }
            other => other,
        }?;
        self.recent_style = id;
        Ok(id)
    }

    /// The id of a multi-code-point cluster, sweeping like [`Self::intern_style`].
    pub fn intern_cluster(&mut self, text: &str) -> Result<ClusterId, GridError> {
        let id = match self.clusters.intern(text) {
            Err(GridError::ClusterTableFull { .. })
                if self.sweep_pays(self.clusters.inserts_since_sweep(), self.clusters.cap()) =>
            {
                self.sweep();
                self.clusters.intern(text)
            }
            other => other,
        }?;
        self.recent_cluster = Some(id);
        Ok(id)
    }

    /// Bumped by every sweep that reclaimed ids. An id held outside the grid
    /// (the cursor's pen) is valid only while the epoch it was interned in
    /// lasts; the most recently interned style and cluster always survive.
    pub fn intern_epoch(&self) -> u64 {
        self.epoch
    }

    /// The style table, for resolving cell styles.
    pub fn styles(&self) -> &StyleTable {
        &self.styles
    }

    /// The cluster table, for resolving cell text.
    pub fn clusters(&self) -> &ClusterTable {
        &self.clusters
    }

    fn phys(&self, logical: usize) -> usize {
        (self.zero + logical) & self.mask
    }

    fn fresh_id(&mut self) -> RowId {
        self.next_id = self.next_id.wrapping_add(1);
        RowId(self.next_id)
    }

    fn advance(&mut self, fill: Cell) {
        let rows = usize::from(self.screen_rows);
        if self.history_limit > 0 {
            let top = self.phys(self.history);
            if let Some(row) = self.rows.get_mut(top) {
                row.compact();
            }
        }
        if self.history < self.history_limit {
            self.history += 1;
        } else {
            let oldest = self.zero;
            self.zero = (self.zero + 1) & self.mask;
            self.released_since_sweep += 1;
            if self.phys(self.history + rows - 1) != oldest
                && let Some(row) = self.rows.get_mut(oldest)
            {
                *row = Row::new(RowId::default(), self.cols, Cell::EMPTY);
            }
        }
        if self.display_offset > 0 {
            self.display_offset = (self.display_offset + 1).min(self.history);
        }
        let bottom = self.phys(self.history + rows - 1);
        self.install(bottom, fill);
    }

    /// Starts a new line in slot `at`, pushing the slot when it is the first
    /// use of that part of the ring.
    fn install(&mut self, at: usize, fill: Cell) {
        let id = self.fresh_id();
        if at >= self.rows.len() {
            let cols = self.cols;
            self.rows
                .resize_with(at + 1, || Row::new(RowId::default(), cols, Cell::EMPTY));
        }
        if let Some(row) = self.rows.get_mut(at) {
            row.recycle(id, fill);
        }
    }

    fn clamp_region(&self, region: Range<u16>) -> (usize, usize) {
        let end = usize::from(region.end.min(self.screen_rows));
        (usize::from(region.start).min(end), end)
    }

    fn swap_screen(&mut self, a: usize, b: usize) {
        let (a, b) = (self.phys(self.history + a), self.phys(self.history + b));
        // Screen slots always exist; the guard keeps a broken invariant from
        // becoming a panic on the reader thread.
        if a.max(b) < self.rows.len() {
            self.rows.swap(a, b);
        }
    }

    fn recycle_screen(&mut self, rows: Range<usize>, fill: Cell) {
        self.released_since_sweep += rows.len();
        for r in rows {
            let at = self.phys(self.history + r);
            self.install(at, fill);
        }
    }

    fn sweep_pays(&self, inserts: usize, cap: usize) -> bool {
        inserts * SWEEP_DIVISOR >= cap
            || self.released_since_sweep * SWEEP_DIVISOR >= self.rows.len()
    }

    fn sweep(&mut self) {
        let mut styles: Marks = self.styles.marks();
        let mut clusters = self.clusters.marks();
        styles.mark(u32::from(self.recent_style.0));
        if let Some(id) = self.recent_cluster {
            clusters.mark(id.0);
        }
        for row in &self.rows {
            row.mark(&mut styles, &mut clusters);
        }
        let freed = self.styles.sweep(&styles) + self.clusters.sweep(&clusters);
        self.released_since_sweep = 0;
        if freed > 0 {
            self.epoch += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Content;
    use proptest::prelude::*;

    const COLS: u16 = 8;
    const SCREEN: u16 = 3;

    fn grid(history: usize) -> Grid {
        Grid::new(COLS, SCREEN, history).unwrap()
    }

    fn ch(c: char) -> Cell {
        Cell::char(c, StyleId::DEFAULT)
    }

    /// Writes `c` at column 0 of the bottom screen row, then scrolls it up.
    fn emit(g: &mut Grid, c: char) {
        let bottom = g.screen_rows() - 1;
        g.screen_row_mut(bottom)
            .unwrap()
            .put(0, ch(c), false)
            .unwrap();
        g.scroll_up(1, Cell::EMPTY);
    }

    fn first_char(row: &Row) -> Option<char> {
        match row.cell(0)?.content() {
            Content::Char(c) => Some(c),
            _ => None,
        }
    }

    fn screen_ids(g: &Grid) -> Vec<RowId> {
        (0..g.screen_rows())
            .map(|r| g.screen_row(r).unwrap().id())
            .collect()
    }

    #[test]
    fn grid_needs_a_row_and_a_column() {
        assert_eq!(Grid::new(0, 1, 0).unwrap_err(), GridError::Empty);
        assert_eq!(Grid::new(1, 0, 0).unwrap_err(), GridError::Empty);
    }

    #[test]
    fn scrollback_request_is_clamped_to_the_ring_cap() {
        let g = Grid::new(COLS, SCREEN, usize::MAX).unwrap();
        assert_eq!(g.history_limit(), MAX_RING_ROWS - usize::from(SCREEN));
        assert_eq!(g.capacity(), MAX_RING_ROWS);
        assert_eq!(g.rows.len(), usize::from(SCREEN), "slots are lazy");
    }

    #[test]
    fn ring_is_a_power_of_two_covering_screen_and_history() {
        let g = grid(10);
        assert_eq!(g.capacity(), 16);
        assert_eq!(g.history_limit(), 10, "the limit is not rounded up");
    }

    #[test]
    fn scrolling_a_full_ring_is_an_offset_change() {
        // Screen 3 plus history 5 fills a ring of 8 exactly.
        let mut g = grid(5);
        for c in 'a'..='z' {
            emit(&mut g, c);
        }
        assert_eq!(g.rows.len(), g.capacity());
        g.screen_row_mut(0).unwrap().put(0, ch('!'), false).unwrap();
        let kept = g.screen_row(0).unwrap().id();
        let kept_cells = g.screen_row(0).unwrap().cells().next();
        let slot = g.phys(g.history_len());
        let zero = g.zero;
        g.scroll_up(1, Cell::EMPTY);
        assert_eq!(g.zero, (zero + 1) & g.mask);
        assert_eq!(g.rows.len(), g.capacity(), "no slot added");
        let moved = g.history_len() - 1;
        assert_eq!(g.phys(moved), slot, "the row stayed in its slot");
        let moved = g.row(moved).unwrap();
        assert_eq!(moved.id(), kept, "the row moved into history by offset");
        assert_eq!(moved.cells().next(), kept_cells);
    }

    #[test]
    fn scroll_keeps_ids_and_order_of_rows() {
        let mut g = grid(4);
        let before = screen_ids(&g);
        emit(&mut g, 'x');
        let after = screen_ids(&g);
        assert_eq!(after[..2], before[1..]);
        assert!(
            !before.contains(&after[2]),
            "the new bottom row has a fresh id"
        );
        assert_eq!(g.row(0).unwrap().id(), before[0]);
    }

    #[test]
    fn history_stops_at_its_limit_and_drops_the_oldest() {
        const LIMIT: usize = 5;
        let mut g = grid(LIMIT);
        for c in 'a'..='t' {
            emit(&mut g, c);
        }
        assert_eq!(g.history_len(), LIMIT);
        let oldest: Vec<_> = (0..LIMIT).filter_map(|i| first_char(g.row(i)?)).collect();
        // 's' and 't' are still on screen; the five before them remain.
        assert_eq!(oldest, ['n', 'o', 'p', 'q', 'r']);
    }

    #[test]
    fn ring_larger_than_needed_keeps_evicted_slots_empty() {
        // Screen 3 plus history 2 needs 5 rows of a ring of 8.
        let mut g = grid(2);
        for c in 'a'..='z' {
            emit(&mut g, c);
        }
        assert!(g.rows.len() <= g.capacity());
        let live: Vec<usize> = (0..g.history_len() + usize::from(SCREEN))
            .map(|i| g.phys(i))
            .collect();
        for (at, row) in g.rows.iter().enumerate() {
            if !live.contains(&at) {
                assert!(row.is_uniform(), "slot {at} still holds cells");
                assert_eq!(row.memory_bytes(), size_of::<Row>());
            }
        }
    }

    #[test]
    fn screen_without_history_recycles_its_own_rows() {
        let mut g = grid(0);
        for c in 'a'..='z' {
            emit(&mut g, c);
        }
        assert_eq!(g.history_len(), 0);
        assert!(g.rows.len() <= g.capacity());
        assert_eq!(first_char(g.screen_row(SCREEN - 2).unwrap()), Some('z'));
    }

    #[test]
    fn rows_entering_history_are_compacted() {
        let mut g = grid(4);
        emit(&mut g, 'a');
        g.scroll_up(SCREEN, Cell::EMPTY);
        let row = g.row(g.history_len() - usize::from(SCREEN) + 1).unwrap();
        assert_eq!(first_char(row), Some('a'));
        assert_eq!(row.memory_bytes(), size_of::<Row>() + size_of::<Cell>());
    }

    #[test]
    fn region_scrolls_rotate_inside_the_region_only() {
        const ROWS: u16 = 5;
        let mut g = Grid::new(COLS, ROWS, 10).unwrap();
        for (r, c) in (0..ROWS).zip('a'..) {
            g.screen_row_mut(r).unwrap().put(0, ch(c), false).unwrap();
        }
        let text = |g: &Grid| -> String {
            (0..ROWS)
                .map(|r| first_char(g.screen_row(r).unwrap()).unwrap_or('.'))
                .collect()
        };
        g.scroll_region_up(1..4, 1, Cell::EMPTY);
        assert_eq!(text(&g), "acd.e");
        g.scroll_region_down(1..4, 2, Cell::EMPTY);
        assert_eq!(text(&g), "a..ce");
        assert_eq!(g.history_len(), 0, "a partial region never feeds history");
        g.scroll_region_up(0..ROWS, 1, Cell::EMPTY);
        assert_eq!(g.history_len(), 1, "the full screen does");
    }

    #[test]
    fn viewport_stays_on_its_content_while_output_scrolls() {
        let mut g = grid(10);
        for c in 'a'..='f' {
            emit(&mut g, c);
        }
        g.scroll_display(2);
        let seen = g.visible_row(0).unwrap().id();
        emit(&mut g, 'g');
        assert_eq!(g.visible_row(0).unwrap().id(), seen);
        assert_eq!(g.display_offset(), 3);
    }

    #[test]
    fn display_offset_is_clamped_to_history() {
        let mut g = grid(10);
        emit(&mut g, 'a');
        g.scroll_display(isize::MAX);
        assert_eq!(g.display_offset(), 1);
        g.scroll_display(isize::MIN);
        assert_eq!(g.display_offset(), 0);
        assert!(g.visible_row(SCREEN).is_none());
    }

    #[test]
    fn clear_history_keeps_the_screen() {
        let mut g = grid(10);
        for c in 'a'..='f' {
            emit(&mut g, c);
        }
        let screen = screen_ids(&g);
        g.scroll_display(2);
        g.clear_history();
        assert_eq!((g.history_len(), g.display_offset()), (0, 0));
        assert_eq!(screen_ids(&g), screen);
        emit(&mut g, 'g');
        assert_eq!(g.history_len(), 1);
    }

    fn style(n: u8) -> Style {
        Style {
            fg: crate::style::Color::Indexed(n),
            ..Style::default()
        }
    }

    #[test]
    fn full_style_table_sweeps_styles_no_row_uses() {
        const CAP: usize = 4;
        let mut g = grid(0).with_tables(StyleTable::with_cap(CAP), ClusterTable::default());
        let used = g.intern_style(&style(1)).unwrap();
        let row = g.screen_row_mut(0).unwrap();
        row.put(0, Cell::char('x', used), false).unwrap();
        let garbage = g.intern_style(&style(2)).unwrap();
        let recent = g.intern_style(&style(3)).unwrap();
        let fresh = g.intern_style(&style(4)).unwrap();
        assert_eq!(fresh, garbage, "the unused id was reclaimed");
        assert_eq!(g.intern_epoch(), 1);
        assert_eq!(g.styles().get(used), Some(&style(1)));
        assert_eq!(
            g.styles().get(recent),
            Some(&style(3)),
            "most recent is pinned"
        );
    }

    #[test]
    fn table_full_of_live_styles_fails_without_sweeping_again() {
        const CAP: usize = 3;
        // More rows than SWEEP_DIVISOR, so one recycled row does not pay.
        const ROWS: u16 = 16;
        let mut g = Grid::new(COLS, ROWS, 100)
            .unwrap()
            .with_tables(StyleTable::with_cap(CAP), ClusterTable::default());
        for (col, n) in (0..).zip(1..) {
            let Ok(id) = g.intern_style(&style(n)) else {
                break;
            };
            let row = g.screen_row_mut(0).unwrap();
            row.put(col, Cell::char('x', id), false).unwrap();
        }
        let full = Err(GridError::StyleTableFull { cap: CAP });
        assert_eq!(g.styles().inserts_since_sweep(), 0, "one sweep ran");
        g.scroll_region_up(1..2, 1, Cell::EMPTY);
        assert_eq!(g.intern_style(&style(9)), full);
        assert_eq!(
            g.released_since_sweep, 1,
            "no second sweep for one recycled row"
        );
    }

    #[test]
    fn clusters_of_evicted_rows_are_reclaimed() {
        const CAP: usize = 1;
        let mut g = grid(0).with_tables(StyleTable::default(), ClusterTable::with_cap(CAP));
        let a = g.intern_cluster("a\u{301}").unwrap();
        let row = g.screen_row_mut(0).unwrap();
        row.put(0, Cell::cluster(a, StyleId::DEFAULT), false)
            .unwrap();
        g.recent_cluster = None;
        assert!(
            g.intern_cluster("e\u{301}").is_err(),
            "row 0 still shows it"
        );
        g.scroll_up(SCREEN, Cell::EMPTY);
        let e = g.intern_cluster("e\u{301}").unwrap();
        assert_eq!(g.clusters().get(e), Some("e\u{301}"));
        assert_eq!(g.clusters().len(), CAP);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Up(u16),
        RegionUp(u16, u16, u16),
        RegionDown(u16, u16, u16),
        Display(i8),
        ClearHistory,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0..5u16).prop_map(Op::Up),
            (0..6u16, 0..6u16, 0..4u16).prop_map(|(a, b, n)| Op::RegionUp(a, b, n)),
            (0..6u16, 0..6u16, 0..4u16).prop_map(|(a, b, n)| Op::RegionDown(a, b, n)),
            any::<i8>().prop_map(Op::Display),
            Just(Op::ClearHistory),
        ]
    }

    proptest! {
        #[test]
        fn any_scroll_sequence_keeps_the_ring_consistent(
            limit in 0..12usize,
            ops in proptest::collection::vec(op(), 0..200),
        ) {
            const ROWS: u16 = 4;
            let mut g = Grid::new(COLS, ROWS, limit).unwrap();
            for op in ops {
                match op {
                    Op::Up(n) => g.scroll_up(n, Cell::EMPTY),
                    Op::RegionUp(a, b, n) => g.scroll_region_up(a..b, n, Cell::EMPTY),
                    Op::RegionDown(a, b, n) => g.scroll_region_down(a..b, n, Cell::EMPTY),
                    Op::Display(d) => g.scroll_display(isize::from(d)),
                    Op::ClearHistory => g.clear_history(),
                }
                prop_assert!(g.history_len() <= g.history_limit());
                prop_assert!(g.display_offset() <= g.history_len());
                prop_assert!(g.rows.len() <= g.capacity());
                let total = g.history_len() + usize::from(ROWS);
                let mut ids: Vec<RowId> = (0..total).map(|i| g.row(i).unwrap().id()).collect();
                prop_assert!(ids.iter().all(|id| *id != RowId::default()));
                ids.sort_unstable();
                ids.dedup();
                prop_assert_eq!(ids.len(), total, "live rows have distinct ids");
            }
        }
    }
}
