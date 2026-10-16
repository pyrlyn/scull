//! The UI-owned frame: a copy of the viewport in flat buffers that the UI
//! reads without touching the terminal. Separate from the terminal so the UI
//! holds the core's lock only while [`Terminal::update_frame`] copies.
//!
//! The update copies only what the damage names (`damage.rs`): rows that
//! moved unchanged are moved inside the frame, as foot replays its scroll
//! damage (`docs/research/foot-contour.md` §1.11), and only dirty rows are
//! read from the grid. Contour rebuilds the whole page on every refresh
//! (§1.12); a native UI behind a C ABI pays for every byte it copies, so
//! the frame keeps the previous picture and patches it.
//!
//! Per row the frame holds fixed-size cells for backgrounds and cursor
//! placement, plus the row's text cut into runs of one style and one cell
//! width: the unit the platform shaper (CoreText, DirectWrite) takes, since
//! the core decides widths and the platform only shapes.
//!
//! Images ride along as placements in viewport cells. Each row also keeps a
//! keyed hash of the image slices drawn on it, so a row whose text moved
//! unchanged is still repainted when the images over it differ (a region
//! scroll slides text under an image anchored to its line).

use std::hash::{BuildHasher, RandomState};
use std::sync::Arc;
use std::time::Instant;

use scull_grid::{Cell, CellFlags, Content, Grid, Style, StyleId};
use scull_image::{Crop, Image};

use crate::damage::{Damage, RowStamp};
use crate::preedit::{FramePreedit, Preedit, PreeditLayout};
use crate::terminal::Terminal;

/// Drawn for a cluster id the table no longer has. Rows keep their
/// clusters alive, so this marks a broken invariant instead of a panic.
const MISSING_CLUSTER: &str = "\u{FFFD}";

/// Columns of a wide character's head cell.
const WIDE_COLS: u8 = 2;

/// One cell as the UI paints it: 8 bytes, laid out for the C ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCell {
    /// The first code point of the cell's text; 0 when it shows none.
    pub codepoint: u32,
    /// The style, read through [`Frame::style`].
    pub style: u16,
    /// Columns the character covers: 1, 2 for a wide head, 0 for the
    /// spacer behind it.
    pub width: u8,
    /// [`FrameCell::CLUSTER`] and the highlight bits, or nothing.
    pub flags: u8,
}

impl FrameCell {
    /// The text has more code points than `codepoint`; the full cluster is
    /// in the row's text runs.
    pub const CLUSTER: u8 = 1;
    /// The cell is selected.
    pub const SELECTED: u8 = 2;
    /// The cell is part of a search match.
    pub const MATCH: u8 = 4;
    /// The cell is part of the current search match (also [`Self::MATCH`]).
    pub const CURRENT_MATCH: u8 = 8;
}

/// Highlight bits laid over columns `start..end` of viewport row `row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) row: u16,
    pub(crate) start: u16,
    pub(crate) end: u16,
    pub(crate) flags: u8,
}

/// Neighbouring cells of one style and one width, with their text, for the
/// platform to shape. Cells without text (blank, erased) end a run and
/// belong to none: they paint only a background.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRun {
    /// First column.
    pub col: u16,
    /// Columns covered, wide spacers included.
    pub cols: u16,
    /// The style, read through [`Frame::style`].
    pub style: u16,
    /// Columns each character covers: 1 or 2.
    pub width: u8,
    /// Byte offset of the run's text in [`FrameRow::text`].
    pub text_start: u32,
    /// Byte length of the run's text.
    pub text_len: u32,
}

/// The text of one frame row and its runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameRow {
    text: String,
    runs: Vec<TextRun>,
}

impl FrameRow {
    /// Every run's text, left to right, UTF-8.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The runs, left to right.
    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }

    /// The text of `run`, empty for a run of another row.
    pub fn run_text(&self, run: &TextRun) -> &str {
        let start = usize::try_from(run.text_start).unwrap_or(usize::MAX);
        let len = usize::try_from(run.text_len).unwrap_or(0);
        self.text
            .get(start..start.saturating_add(len))
            .unwrap_or_default()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.runs.clear();
    }

    /// Appends `cell` at `col` and returns what the cell buffer holds.
    fn push(&mut self, col: u16, cell: Cell, grid: &Grid) -> FrameCell {
        let style = cell.style().0;
        let flags = cell.flags();
        if flags.contains(CellFlags::SPACER) {
            return FrameCell {
                style,
                ..FrameCell::default()
            };
        }
        let width = if flags.contains(CellFlags::WIDE) {
            WIDE_COLS
        } else {
            1
        };
        let start = self.text.len();
        let (codepoint, cell_flags) = match cell.content() {
            Content::Empty => {
                return FrameCell {
                    style,
                    width,
                    ..FrameCell::default()
                };
            }
            Content::Char(c) => {
                self.text.push(c);
                (u32::from(c), 0)
            }
            Content::Cluster(id) => {
                let text = grid.clusters().get(id).unwrap_or(MISSING_CLUSTER);
                self.text.push_str(text);
                let first = text.chars().next().map_or(0, u32::from);
                (first, FrameCell::CLUSTER)
            }
        };
        self.extend_run(col, style, width, start);
        FrameCell {
            codepoint,
            style,
            width,
            flags: cell_flags,
        }
    }

    /// Appends the preedit's clusters at its columns, in the default style.
    fn push_preedit(&mut self, p: &PreeditLayout, text: &str) {
        let mut col = p.col;
        for (range, width) in &p.clusters {
            let start = self.text.len();
            self.text
                .push_str(text.get(range.clone()).unwrap_or_default());
            self.extend_run(col, StyleId::DEFAULT.0, *width, start);
            col = col.saturating_add(u16::from(*width));
        }
    }

    /// Adds the text from byte `start` to the last run, or opens a run.
    fn extend_run(&mut self, col: u16, style: u16, width: u8, start: usize) {
        let len = u32::try_from(self.text.len() - start).unwrap_or(u32::MAX);
        if let Some(last) = self.runs.last_mut()
            && last.style == style
            && last.width == width
            && last.col.saturating_add(last.cols) == col
        {
            last.cols = last.cols.saturating_add(u16::from(width));
            last.text_len = last.text_len.saturating_add(len);
            return;
        }
        self.runs.push(TextRun {
            col,
            cols: u16::from(width),
            style,
            width,
            text_start: u32::try_from(start).unwrap_or(u32::MAX),
            text_len: len,
        });
    }
}

/// Where the UI draws the cursor, in viewport rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCursor {
    /// Viewport row.
    pub row: u16,
    /// Column.
    pub col: u16,
}

/// One image shown in the viewport. The host scales the `crop` of the
/// image to the `cols` x `rows` cell rectangle (sixel and auto-sized iTerm2
/// images are padded to whole cells, so they show at their own size) and
/// draws placements in `z` order, negative ones below the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramePlacement {
    /// The pixels, kept alive by the frame until its next update. A texture
    /// cache keys on the pointer and [`Image::generation`].
    pub image: Arc<Image>,
    /// Viewport row of the top-left cell; negative when the image starts
    /// above the viewport.
    pub row: i64,
    /// Column of the top-left cell.
    pub col: u32,
    /// Width in cells.
    pub cols: u32,
    /// Height in cells.
    pub rows: u32,
    /// Pixel offset inside the top-left cell.
    pub offset_x: u32,
    /// Pixel offset inside the top-left cell.
    pub offset_y: u32,
    /// The part of the image shown, in image pixels.
    pub crop: Crop,
    /// Stacking order; negative draws below the text.
    pub z: i32,
}

/// The UI's copy of the viewport. Create one per view and keep it: every
/// update reuses its buffers and copies only what changed.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    cols: u16,
    rows: u16,
    alt: bool,
    filled: bool,
    cells: Vec<FrameCell>,
    lines: Vec<FrameRow>,
    stamps: Vec<RowStamp>,
    styles: Vec<Style>,
    cursor: Option<FrameCursor>,
    damage: Damage,
    old_stamps: Vec<RowStamp>,
    spare_cells: Vec<FrameCell>,
    spare_lines: Vec<FrameRow>,
    placements: Vec<FramePlacement>,
    /// Per row, a hash of the image slices on it (0 for none), now and in
    /// the previous picture.
    slices: Vec<u64>,
    old_slices: Vec<u64>,
    /// Keyed per frame, so a program cannot aim for a collision that would
    /// leave a stale image slice on screen.
    hasher: RandomState,
    preedit: Preedit,
    /// Where the last update laid the preedit; it rides in the same row
    /// hash as the images, so damage repaints its row and never scrolls it.
    shown_preedit: Option<PreeditLayout>,
    /// Selection and search highlights by row; they ride in the row hash
    /// too, so a row repaints when its highlights change.
    marks: Vec<Mark>,
}

impl Frame {
    /// An empty frame; the first update fills it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Columns.
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Rows.
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// All cells, row after row.
    pub fn cells(&self) -> &[FrameCell] {
        &self.cells
    }

    /// The cells of row `r`.
    pub fn row_cells(&self, r: u16) -> &[FrameCell] {
        let cols = usize::from(self.cols);
        let at = usize::from(r) * cols;
        self.cells.get(at..at + cols).unwrap_or_default()
    }

    /// The text and runs of row `r`.
    pub fn row(&self, r: u16) -> Option<&FrameRow> {
        self.lines.get(usize::from(r))
    }

    /// The id and generation of the line row `r` shows.
    pub fn stamp(&self, r: u16) -> Option<RowStamp> {
        self.stamps.get(usize::from(r)).copied()
    }

    /// The style behind a cell's or run's style id.
    pub fn style(&self, id: u16) -> Option<&Style> {
        self.styles.get(usize::from(id))
    }

    /// The cursor, `None` when hidden or outside the viewport.
    pub fn cursor(&self) -> Option<FrameCursor> {
        self.cursor
    }

    /// The images in the viewport, lowest `z` first.
    pub fn placements(&self) -> &[FramePlacement] {
        &self.placements
    }

    /// What the last update changed: blit the scrolls from the previous
    /// picture, then repaint the dirty rows.
    pub fn damage(&self) -> &Damage {
        &self.damage
    }

    /// Sets the input method's composing text and its caret (a byte offset
    /// into `text`); empty text clears it. The next update lays it over
    /// the cursor row as text of the default style and moves the cursor to
    /// the caret. Cut at [`crate::MAX_PREEDIT_BYTES`].
    pub fn set_preedit(&mut self, text: &str, caret: usize) {
        self.preedit.set(text, caret);
    }

    /// Where the last update showed the preedit, `None` when it did not.
    pub fn preedit(&self) -> Option<FramePreedit> {
        self.shown_preedit.as_ref().map(|p| FramePreedit {
            row: p.row,
            col: p.col,
            cols: p.cols,
        })
    }

    /// Copies what changed in `term`'s viewport since the last update.
    fn copy_from(&mut self, term: &Terminal) {
        let grid = term.grid();
        let (cols, rows) = (grid.cols(), grid.screen_rows());
        let resized = (cols, rows) != (self.cols, self.rows);
        // Row ids are unique per grid only, so ids on the other screen say
        // nothing about this one.
        let full = !self.filled || resized || self.alt != term.is_alt_screen();
        if resized {
            let size = usize::from(cols) * usize::from(rows);
            self.cells.clear();
            self.cells.resize(size, FrameCell::default());
            self.lines.resize_with(usize::from(rows), FrameRow::default);
        }
        (self.cols, self.rows) = (cols, rows);
        self.alt = term.is_alt_screen();
        self.filled = true;
        std::mem::swap(&mut self.old_stamps, &mut self.stamps);
        self.stamps.clear();
        self.stamps.extend((0..rows).map(|r| {
            grid.visible_row(r)
                .map_or_else(RowStamp::default, RowStamp::of)
        }));
        std::mem::swap(&mut self.old_slices, &mut self.slices);
        self.collect_placements(term);
        self.lay_out_preedit(term);
        self.collect_marks(term);
        let (now, before) = (&self.slices, &self.old_slices);
        let same = |r: u16, from: u16| now.get(usize::from(r)) == before.get(usize::from(from));
        self.damage
            .compute_with(&self.old_stamps, &self.stamps, full, same);
        self.apply_scrolls();
        let mut dirty = std::mem::take(&mut self.damage);
        for &r in dirty.dirty() {
            self.paint(grid, r);
        }
        std::mem::swap(&mut self.damage, &mut dirty);
        self.cursor = match &self.shown_preedit {
            Some(p) => Some(FrameCursor {
                row: p.row,
                col: p.caret,
            }),
            None => cursor_in_viewport(term).filter(|_| term.modes().cursor_visible),
        };
    }

    /// Lays the preedit out at the cursor, even a hidden one: composing
    /// needs a caret. Off the viewport it is not shown.
    fn lay_out_preedit(&mut self, term: &Terminal) {
        let (width, policy) = (term.state.width, term.state.policy());
        self.shown_preedit = cursor_in_viewport(term).and_then(|c| {
            self.preedit
                .layout((c.row, c.col), self.cols, width, policy)
        });
        if let Some(p) = &self.shown_preedit
            && let Some(slot) = self.slices.get_mut(usize::from(p.row))
        {
            *slot = self.hasher.hash_one((*slot, self.preedit.text(), p));
        }
    }

    fn collect_marks(&mut self, term: &Terminal) {
        self.marks.clear();
        term.selection_marks(&mut self.marks);
        term.search_marks(&mut self.marks);
        self.marks.sort_by_key(|m| m.row);
        for m in &self.marks {
            if let Some(slot) = self.slices.get_mut(usize::from(m.row)) {
                *slot = self.hasher.hash_one((*slot, m.start, m.end, m.flags));
            }
        }
    }

    /// Collects the placements in the viewport and hashes the slice of
    /// each that every row shows.
    fn collect_placements(&mut self, term: &Terminal) {
        let shown = i64::from(self.rows);
        self.placements.clear();
        self.slices.clear();
        self.slices.resize(usize::from(self.rows), 0);
        let back = u64::try_from(term.grid().display_offset()).unwrap_or(u64::MAX);
        // Absolute lines stay far below `i64::MAX`: one per line ever output.
        let top = i64::try_from(term.screen_top_line().saturating_sub(back)).unwrap_or(i64::MAX);
        let store = term.images();
        for p in store.placements() {
            let Some(image) = store.peek(p.image) else {
                continue;
            };
            let row = i64::try_from(p.row).unwrap_or(i64::MAX).saturating_sub(top);
            let end = row.saturating_add(i64::from(p.rows));
            if row >= shown || end <= 0 {
                continue;
            }
            let crop = (p.crop.x, p.crop.y, p.crop.width, p.crop.height);
            let cell = (p.col, p.cols, p.rows, p.offset_x, p.offset_y, p.z);
            let pixels = (Arc::as_ptr(image).addr(), image.generation());
            for r in row.max(0)..end.min(shown) {
                let slice = self.hasher.hash_one((pixels, cell, crop, r - row));
                if let Some(at) = usize::try_from(r).ok().and_then(|r| self.slices.get_mut(r)) {
                    *at = self.hasher.hash_one((*at, slice));
                }
            }
            self.placements.push(FramePlacement {
                image: Arc::clone(image),
                row,
                col: p.col,
                cols: p.cols,
                rows: p.rows,
                offset_x: p.offset_x,
                offset_y: p.offset_y,
                crop: p.crop,
                z: p.z,
            });
        }
        self.placements.sort_by_key(|p| p.z);
    }

    /// Moves the rows the damage says moved. Every scroll reads the
    /// previous picture, so sources are set aside before any is written.
    fn apply_scrolls(&mut self) {
        let cols = usize::from(self.cols);
        let Self {
            cells,
            lines,
            damage,
            spare_cells,
            spare_lines,
            ..
        } = self;
        spare_cells.clear();
        spare_lines.clear();
        for s in damage.scrolls() {
            let from = usize::from(s.from);
            let len = usize::from(s.end - s.start);
            if let Some(src) = cells.get(from * cols..(from + len) * cols) {
                spare_cells.extend_from_slice(src);
            }
            if let Some(src) = lines.get_mut(from..from + len) {
                spare_lines.extend(src.iter_mut().map(std::mem::take));
            }
        }
        let (mut cell_at, mut line_at) = (0, 0);
        for s in damage.scrolls() {
            let start = usize::from(s.start);
            let len = usize::from(s.end - s.start);
            let n = len * cols;
            if let (Some(dst), Some(src)) = (
                cells.get_mut(start * cols..start * cols + n),
                spare_cells.get(cell_at..cell_at + n),
            ) {
                dst.copy_from_slice(src);
            }
            cell_at += n;
            if let (Some(dst), Some(src)) = (
                lines.get_mut(start..start + len),
                spare_lines.get_mut(line_at..line_at + len),
            ) {
                dst.swap_with_slice(src);
            }
            line_at += len;
        }
    }

    /// Copies viewport row `r` from the grid.
    fn paint(&mut self, grid: &Grid, r: u16) {
        let cols = usize::from(self.cols);
        let at = usize::from(r) * cols;
        let Self {
            cells,
            lines,
            styles,
            preedit,
            shown_preedit,
            marks,
            ..
        } = self;
        let (Some(cells), Some(line)) =
            (cells.get_mut(at..at + cols), lines.get_mut(usize::from(r)))
        else {
            return;
        };
        line.clear();
        let Some(row) = grid.visible_row(r) else {
            cells.fill(FrameCell::default());
            return;
        };
        let over = shown_preedit.as_ref().filter(|p| p.row == r);
        let mut seen = None;
        for ((col, slot), cell) in (0..=u16::MAX).zip(cells.iter_mut()).zip(row.cells()) {
            if seen != Some(cell.style()) {
                seen = Some(cell.style());
                refresh_style(styles, grid, cell.style());
            }
            if let Some(p) = over {
                if p.covers(col) {
                    if col == p.col {
                        line.push_preedit(p, preedit.text());
                    }
                    continue;
                }
                if col.saturating_add(1) == p.col && cell.flags().contains(CellFlags::WIDE) {
                    // Its right half is under the preedit: show neither.
                    *slot = FrameCell {
                        style: cell.style().0,
                        width: 1,
                        ..FrameCell::default()
                    };
                    continue;
                }
            }
            *slot = line.push(col, cell, grid);
        }
        if let Some(p) = over {
            fill_preedit(p, preedit.text(), cells);
        }
        let first = marks.partition_point(|m| m.row < r);
        for m in marks.iter().skip(first).take_while(|m| m.row == r) {
            let end = usize::from(m.end).min(cells.len());
            for cell in cells.get_mut(usize::from(m.start)..end).unwrap_or_default() {
                cell.flags |= m.flags;
            }
        }
    }
}

/// Writes the preedit's cells over a row's `cells`.
fn fill_preedit(p: &PreeditLayout, text: &str, cells: &mut [FrameCell]) {
    let mut col = usize::from(p.col);
    for (range, width) in &p.clusters {
        let mut chars = text.get(range.clone()).unwrap_or_default().chars();
        let head = FrameCell {
            codepoint: chars.next().map_or(0, u32::from),
            style: StyleId::DEFAULT.0,
            width: *width,
            flags: if chars.next().is_some() {
                FrameCell::CLUSTER
            } else {
                0
            },
        };
        let tail = usize::from(*width == WIDE_COLS);
        if let Some(slots) = cells.get_mut(col..=col + tail) {
            slots.fill(FrameCell::default());
            if let Some(first) = slots.first_mut() {
                *first = head;
            }
        }
        col += usize::from(*width);
    }
}

/// Copies style `id` into the frame's table. Ids of unchanged rows stay
/// valid: the grid only reclaims ids no row uses, so refreshing the ids of
/// the rows copied keeps the whole table right.
fn refresh_style(styles: &mut Vec<Style>, grid: &Grid, id: StyleId) {
    let at = usize::from(id.0);
    if styles.len() <= at {
        styles.resize(at + 1, Style::default());
    }
    if let Some(slot) = styles.get_mut(at) {
        *slot = grid.styles().get(id).copied().unwrap_or_default();
    }
}

/// The cursor in viewport rows, shown or not, when the viewport is not
/// scrolled back past it.
fn cursor_in_viewport(term: &Terminal) -> Option<FrameCursor> {
    let grid = term.grid();
    let cursor = term.cursor();
    let row = usize::from(cursor.row).checked_add(grid.display_offset())?;
    let row = u16::try_from(row)
        .ok()
        .filter(|&r| r < grid.screen_rows())?;
    Some(FrameCursor {
        row,
        col: cursor.col,
    })
}

impl Terminal {
    /// Brings `frame` up to date with the viewport: the only call that
    /// copies out of the terminal, so a UI holds the terminal's lock just
    /// for it. While synchronized output holds the picture (see
    /// [`Self::sync_held`]) the frame is left as it is with no damage and
    /// this returns false.
    pub fn update_frame(&mut self, frame: &mut Frame, now: Instant) -> bool {
        if self.sync_held(now) {
            frame.damage.clear();
            return false;
        }
        frame.copy_from(self);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;
    use scull_grid::Color;

    use super::*;
    use crate::damage::Scroll;

    const COLS: u16 = 8;
    const ROWS: u16 = 4;

    fn term() -> Terminal {
        Terminal::new(COLS, ROWS, 10).unwrap()
    }

    fn updated(term: &mut Terminal, frame: &mut Frame) {
        assert!(term.update_frame(frame, Instant::now()));
    }

    fn fresh(term: &mut Terminal) -> Frame {
        let mut frame = Frame::new();
        updated(term, &mut frame);
        frame
    }

    fn runs(frame: &Frame, r: u16) -> Vec<(u16, u16, u8, String)> {
        let row = frame.row(r).unwrap();
        row.runs()
            .iter()
            .map(|run| (run.col, run.cols, run.width, row.run_text(run).to_owned()))
            .collect()
    }

    fn run(col: u16, cols: u16, width: u8, text: &str) -> (u16, u16, u8, String) {
        (col, cols, width, text.to_owned())
    }

    #[test]
    fn runs_split_on_style_width_and_blank_cells() {
        let mut t = term();
        t.feed("ab\x1b[31mc\x1b[m\u{4e2d}\u{6587}d e".as_bytes());
        let f = fresh(&mut t);
        assert_eq!(
            runs(&f, 0),
            [
                run(0, 2, 1, "ab"),
                run(2, 1, 1, "c"),
                run(3, 4, 2, "\u{4e2d}\u{6587}"),
                run(7, 1, 1, "d"),
            ],
            "the space and the rest past the screen edge wrapped"
        );
        let red = f.row_cells(0)[2].style;
        assert_eq!(f.style(red).unwrap().fg, Color::Indexed(1));
        assert_eq!(f.row(0).unwrap().text(), "abc\u{4e2d}\u{6587}d");
        assert_eq!(runs(&f, 1), [run(0, 2, 1, " e")], "a typed space is text");
    }

    #[test]
    fn erased_cells_end_a_run_and_keep_their_background() {
        let mut t = term();
        t.feed(b"ab\x1b[44m\x1b[X\x1b[m\x1b[2Cc");
        let f = fresh(&mut t);
        assert_eq!(runs(&f, 0), [run(0, 2, 1, "ab"), run(4, 1, 1, "c")]);
        let blank = f.row_cells(0)[2];
        assert_eq!((blank.codepoint, blank.width), (0, 1));
        assert_eq!(f.style(blank.style).unwrap().bg, Color::Indexed(4));
    }

    #[test]
    fn wide_cells_are_a_head_and_a_spacer() {
        let mut t = term();
        t.feed("\u{4e2d}".as_bytes());
        let f = fresh(&mut t);
        let cells = f.row_cells(0);
        assert_eq!((cells[0].codepoint, cells[0].width), (0x4e2d, 2));
        assert_eq!((cells[1].codepoint, cells[1].width), (0, 0));
    }

    #[test]
    fn a_cluster_keeps_its_whole_text_in_the_run() {
        let mut t = term();
        t.feed("\x1b[?2027he\u{301}x".as_bytes());
        let f = fresh(&mut t);
        let cell = f.row_cells(0)[0];
        assert_eq!(cell.codepoint, u32::from('e'));
        assert_eq!(cell.flags, FrameCell::CLUSTER);
        assert_eq!(runs(&f, 0), [run(0, 2, 1, "e\u{301}x")]);
    }

    #[test]
    fn the_first_update_and_a_screen_switch_are_full() {
        let mut t = term();
        let mut f = Frame::new();
        updated(&mut t, &mut f);
        assert!(f.damage().is_full());
        updated(&mut t, &mut f);
        assert!(f.damage().is_empty());
        t.feed(b"\x1b[?1049h");
        updated(&mut t, &mut f);
        assert!(f.damage().is_full(), "ids of the other screen collide");
    }

    #[test]
    fn a_scroll_moves_rows_inside_the_frame() {
        let mut t = term();
        t.feed(b"a\r\nb\r\nc\r\nd");
        let mut f = fresh(&mut t);
        t.feed(b"\r\ne");
        updated(&mut t, &mut f);
        let up = Scroll {
            start: 0,
            end: 3,
            from: 1,
        };
        assert_eq!(f.damage().scrolls(), [up]);
        assert_eq!(f.damage().dirty(), [3]);
        let text: Vec<&str> = (0..ROWS).map(|r| f.row(r).unwrap().text()).collect();
        assert_eq!(text, ["b", "c", "d", "e"]);
        assert_eq!(f.row_cells(0)[0].codepoint, u32::from('b'));
    }

    #[test]
    fn a_held_picture_stays_until_the_update_ends() {
        let start = Instant::now();
        let mut t = term();
        let mut f = Frame::new();
        assert!(t.update_frame(&mut f, start));
        t.feed(b"\x1b[?2026hhalf");
        assert!(!t.update_frame(&mut f, start));
        assert!(f.damage().is_empty());
        assert_eq!(f.row(0).unwrap().text(), "");
        t.feed(b" ok\x1b[?2026l");
        assert!(t.update_frame(&mut f, start));
        assert_eq!(f.row(0).unwrap().text(), "half ok");
    }

    #[test]
    fn a_held_picture_is_released_by_the_time_cap() {
        let start = Instant::now();
        let mut t = term();
        let mut f = Frame::new();
        t.feed(b"\x1b[?2026hstuck");
        assert!(!t.update_frame(&mut f, start));
        assert!(t.update_frame(&mut f, start + crate::MAX_SYNC_HOLD));
        assert_eq!(f.row(0).unwrap().text(), "stuck");
    }

    #[test]
    fn the_cursor_hides_with_dectcem_and_off_the_viewport() {
        let mut t = term();
        t.feed(b"\x1b[2;3H");
        let f = fresh(&mut t);
        assert_eq!(f.cursor(), Some(FrameCursor { row: 1, col: 2 }));
        t.feed(b"\x1b[?25l");
        assert_eq!(fresh(&mut t).cursor(), None);
        t.feed(b"\x1b[?25h\x1b[4H\r\n\r\n\r\n");
        t.scroll_display(1);
        assert_eq!(fresh(&mut t).cursor(), None);
    }

    fn text_of(frame: &Frame) -> Vec<&str> {
        (0..ROWS).map(|r| frame.row(r).unwrap().text()).collect()
    }

    #[test]
    fn the_preedit_is_row_text_at_the_cursor_with_the_cursor_at_its_caret() {
        let mut t = term();
        t.feed(b"ab");
        let mut f = fresh(&mut t);
        f.set_preedit("日本", "日本".len());
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [0]);
        assert_eq!(runs(&f, 0), [run(0, 2, 1, "ab"), run(2, 4, 2, "日本")]);
        let cells = f.row_cells(0);
        assert_eq!(
            (cells[2].codepoint, cells[2].width, cells[2].style),
            (0x65e5, 2, 0)
        );
        assert_eq!((cells[3].codepoint, cells[3].width), (0, 0));
        assert_eq!(f.cursor(), Some(FrameCursor { row: 0, col: 6 }));
        let shown = FramePreedit {
            row: 0,
            col: 2,
            cols: 4,
        };
        assert_eq!(f.preedit(), Some(shown));
        updated(&mut t, &mut f);
        assert!(f.damage().is_empty(), "an unchanged preedit is not damage");
        f.set_preedit("", 0);
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [0]);
        assert_eq!(runs(&f, 0), [run(0, 2, 1, "ab")]);
        assert_eq!(
            (f.preedit(), f.cursor()),
            (None, Some(FrameCursor { row: 0, col: 2 }))
        );
    }

    #[test]
    fn the_preedit_hides_the_cells_under_it_and_a_wide_neighbour() {
        let mut t = term();
        t.feed("a\u{4e2d}bcd\x1b[1;3H".as_bytes());
        let mut f = fresh(&mut t);
        f.set_preedit("xy", 1);
        updated(&mut t, &mut f);
        assert_eq!(f.row(0).unwrap().text(), "axycd");
        assert_eq!(
            f.row_cells(0)[1].codepoint,
            0,
            "half a wide character is hidden"
        );
        assert_eq!(f.cursor(), Some(FrameCursor { row: 0, col: 3 }));
    }

    #[test]
    fn a_scroll_leaves_the_preedit_on_the_cursor_row() {
        let mut t = term();
        t.feed(b"a\r\nb\r\nc\r\nd");
        let mut f = fresh(&mut t);
        f.set_preedit("P", 1);
        updated(&mut t, &mut f);
        assert_eq!(text_of(&f), ["a", "b", "c", "dP"]);
        t.feed(b"\r\ne");
        updated(&mut t, &mut f);
        assert_eq!(text_of(&f), ["b", "c", "d", "eP"]);
        assert_eq!(
            f.damage().dirty(),
            [2, 3],
            "the row that had it is repainted"
        );
    }

    #[test]
    fn a_hidden_cursor_still_gets_a_caret_and_history_hides_the_preedit() {
        let mut t = term();
        t.feed(b"\x1b[?25l");
        let mut f = fresh(&mut t);
        f.set_preedit("x", 0);
        updated(&mut t, &mut f);
        assert_eq!(f.cursor(), Some(FrameCursor { row: 0, col: 0 }));
        t.feed(b"\x1b[?25h\x1b[4H\r\n\r\n");
        t.scroll_display(1);
        updated(&mut t, &mut f);
        assert_eq!((f.preedit(), f.cursor()), (None, None));
    }

    /// A kitty image of 2 x 2 pixels, shown `C=1` at the cursor in 2 x 1
    /// cells.
    const KITTY_STILL: &[u8] = b"\x1b_Gi=1,f=24,s=2,v=2,a=T,C=1,c=2,r=1,q=2;AAAAAAAAAAAAAAAA\x1b\\";

    #[test]
    fn placements_are_in_viewport_rows() {
        let mut t = term();
        t.feed(b"\x1b[2;3H");
        t.feed(KITTY_STILL);
        let f = fresh(&mut t);
        let [p] = f.placements() else {
            panic!("{:?}", f.placements());
        };
        assert_eq!((p.row, p.col, p.cols, p.rows), (1, 2, 2, 1));
        assert_eq!((p.image.width(), p.image.height()), (2, 2));
        t.feed(b"\x1b[4H\r\n\r\n");
        assert_eq!(fresh(&mut t).placements().len(), 0, "scrolled off");
        t.scroll_display(2);
        assert_eq!(
            fresh(&mut t).placements()[0].row,
            1,
            "in the scrollback view"
        );
    }

    #[test]
    fn an_image_damages_the_rows_it_covers_though_the_text_is_unchanged() {
        let mut t = term();
        t.feed(b"a\r\nb\x1b[2;1H");
        let mut f = fresh(&mut t);
        t.feed(KITTY_STILL);
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [1]);
        t.feed(b"\x1b_Ga=d,q=2\x1b\\");
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [1], "and the rows it leaves");
        assert!(f.placements().is_empty());
    }

    #[test]
    fn text_scrolled_under_a_fixed_image_is_repainted() {
        let mut t = term();
        t.feed(b"a\r\nb\r\nc\x1b[2;1H");
        t.feed(KITTY_STILL);
        let mut f = fresh(&mut t);
        // A region scroll moves the text but not the image on line 1.
        t.feed(b"\x1b[1;3r\x1b[S");
        updated(&mut t, &mut f);
        assert!(f.damage().dirty().contains(&0), "b moved out from under it");
        assert!(f.damage().dirty().contains(&1), "c moved under it");
    }

    /// One image slice on a row: which pixels (pointer and generation),
    /// which image row of cells, where and how it is drawn.
    type Slice = (
        usize,
        u64,
        i64,
        u32,
        u32,
        u32,
        u32,
        u32,
        (u32, u32, u32, u32),
        i32,
    );

    fn flags(f: &Frame, r: u16) -> Vec<u8> {
        f.row_cells(r).iter().map(|c| c.flags).collect()
    }

    #[test]
    fn selected_cells_are_flagged_and_only_their_row_repaints() {
        let mut t = term();
        t.feed(b"abc\r\ndef");
        let mut f = fresh(&mut t);
        let at = t.viewport_point(1, 1);
        t.select_start(crate::SelectionKind::Cell, at);
        t.select_extend(t.viewport_point(1, 2));
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [1]);
        const S: u8 = FrameCell::SELECTED;
        assert_eq!(flags(&f, 1), [0, S, S, 0, 0, 0, 0, 0]);
        t.select_clear();
        updated(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [1]);
        assert_eq!(flags(&f, 1), [0; 8]);
    }

    #[test]
    fn a_selection_scrolled_with_its_text_keeps_its_flags() {
        let mut t = term();
        t.feed(b"a\r\nb\r\nc\r\nd");
        let at = t.viewport_point(1, 0);
        t.select_start(crate::SelectionKind::Line, at);
        let mut f = fresh(&mut t);
        t.feed(b"\r\n");
        updated(&mut t, &mut f);
        assert_eq!(f.damage().scrolls().len(), 1, "moved, not repainted");
        assert!(flags(&f, 0).iter().all(|&b| b == FrameCell::SELECTED));
        assert!(flags(&f, 1).iter().all(|&b| b == 0));
    }

    #[test]
    fn matches_are_flagged_and_the_current_one_stands_out() {
        let mut t = term();
        t.feed("ab \u{4e2d} ab\r\nab".as_bytes());
        assert!(t.search_set("AB", false));
        let mut f = fresh(&mut t);
        const M: u8 = FrameCell::MATCH;
        assert_eq!(flags(&f, 0), [M, M, 0, 0, 0, 0, M, M]);
        assert_eq!(t.search_next(None).map(|m| m.start.col), Some(0));
        updated(&mut t, &mut f);
        const C: u8 = M | FrameCell::CURRENT_MATCH;
        assert_eq!(flags(&f, 0), [C, C, 0, 0, 0, 0, M, M]);
        assert!(t.search_set("\u{4e2d}", true));
        updated(&mut t, &mut f);
        assert_eq!(
            flags(&f, 0),
            [0, 0, 0, M, M, 0, 0, 0],
            "a wide match covers its spacer"
        );
        assert_eq!(f.damage().dirty(), [0, 1]);
    }

    #[test]
    fn a_match_from_above_the_viewport_is_flagged_where_it_shows() {
        let mut t = term();
        t.feed(b"......abcd\r\n1\r\n2\r\n3");
        assert_eq!(t.grid().history_len(), 1, "the \"ab\" half is in history");
        assert!(t.search_set("abcd", true));
        let f = fresh(&mut t);
        const M: u8 = FrameCell::MATCH;
        assert_eq!(flags(&f, 0), [M, M, 0, 0, 0, 0, 0, 0]);
    }

    /// A row as a UI would put it on screen: every cell with its style
    /// resolved, every run with its text, and the image slices over it.
    #[derive(Clone, Debug, PartialEq)]
    struct Painted {
        cells: Vec<(u32, u8, u8, Option<Style>)>,
        runs: Vec<(u16, u16, u8, Option<Style>, String)>,
        images: Vec<Slice>,
    }

    fn paint(f: &Frame, r: u16) -> Painted {
        let row = f.row(r).unwrap();
        Painted {
            cells: f
                .row_cells(r)
                .iter()
                .map(|c| (c.codepoint, c.width, c.flags, f.style(c.style).copied()))
                .collect(),
            runs: row
                .runs()
                .iter()
                .map(|run| {
                    let style = f.style(run.style).copied();
                    let text = row.run_text(run).to_owned();
                    (run.col, run.cols, run.width, style, text)
                })
                .collect(),
            images: f
                .placements()
                .iter()
                .filter(|p| (p.row..p.row + i64::from(p.rows)).contains(&i64::from(r)))
                .map(|p| {
                    let c = p.crop;
                    (
                        Arc::as_ptr(&p.image).addr(),
                        p.image.generation(),
                        i64::from(r) - p.row,
                        p.col,
                        p.cols,
                        p.rows,
                        p.offset_x,
                        p.offset_y,
                        (c.x, c.y, c.width, c.height),
                        p.z,
                    )
                })
                .collect(),
        }
    }

    fn paint_all(f: &Frame) -> Vec<Painted> {
        (0..f.rows()).map(|r| paint(f, r)).collect()
    }

    /// What a UI does with the damage: blit the scrolls from its last
    /// picture, then repaint the dirty rows.
    fn repaint(canvas: &mut Vec<Painted>, f: &Frame) {
        let d = f.damage();
        if d.is_full() {
            *canvas = paint_all(f);
            return;
        }
        let last = canvas.clone();
        for s in d.scrolls() {
            for (r, from) in s.rows().zip(s.from..) {
                canvas[usize::from(r)] = last[usize::from(from)].clone();
            }
        }
        for &r in d.dirty() {
            canvas[usize::from(r)] = paint(f, r);
        }
    }

    /// Whole sequences the fragments rarely assemble: synchronized output,
    /// screens, regions, colours, clusters, erases and images.
    const SEQUENCES: &[&[u8]] = &[
        b"\x1bPq#0;2;100;0;0#0~~-~~\x1b\\",
        b"\x1b]1337;File=inline=1;width=2;doNotMoveCursor=1:iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\x07",
        KITTY_STILL,
        b"\x1b_Gi=2,f=24,s=2,v=2,a=T,r=2,z=-1,q=2;AAAAAAAAAAAAAAAA\x1b\\",
        b"\x1b_Ga=p,i=1,X=3,Y=5,x=1,w=1,q=2\x1b\\",
        b"\x1b_Ga=d,q=2\x1b\\",
        b"\x1b_Ga=d,d=i,i=2,q=2\x1b\\",
        b"\x1b[S",
        b"\x1b[T",
        b"\x1b[?2026h",
        b"\x1b[?2026l",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[2;3r",
        b"\x1b[r",
        b"\x1b[31m",
        b"\x1b[44m",
        b"\x1b[m",
        b"\x1b[?2027h",
        b"\x1b[2J",
        b"\x1b[3J",
        b"\x1b[L",
        b"\x1b[M",
        b"\x1bM",
        b"\r\n",
    ];

    #[derive(Clone, Debug)]
    enum Step {
        Feed(Vec<u8>),
        View(i8),
        Select(u8, u16, u16),
        Extend(u16, u16),
        Search(&'static str),
        Next(bool),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            4 => crate::terminal::tests::stream().prop_map(Step::Feed),
            4 => proptest::collection::vec(proptest::sample::select(SEQUENCES), 1..6)
                .prop_map(|s| Step::Feed(s.concat())),
            1 => any::<i8>().prop_map(Step::View),
            1 => (0..4u8, 0..8u16, 0..18u16).prop_map(|(k, r, c)| Step::Select(k, r, c)),
            1 => (0..8u16, 0..18u16).prop_map(|(r, c)| Step::Extend(r, c)),
            1 => proptest::sample::select(&["1", "ab", "\u{4e2d}", "x y", ""][..]).prop_map(Step::Search),
            1 => any::<bool>().prop_map(Step::Next),
        ]
    }

    /// Longest pause between steps: past the hold cap, so some steps
    /// release a hold by time.
    const MAX_PAUSE_MS: u64 = 200;

    proptest! {
        #[test]
        fn repainting_the_damage_over_the_last_picture_equals_a_full_repaint(
            cols in 1u16..16,
            rows in 1u16..7,
            steps in proptest::collection::vec((step(), 0..MAX_PAUSE_MS), 1..24),
        ) {
            let mut now = Instant::now();
            let mut t = Terminal::new(cols, rows, 6).unwrap();
            let mut frame = Frame::new();
            let mut canvas = Vec::new();
            for (step, pause) in steps {
                match step {
                    Step::Feed(bytes) => t.feed(&bytes),
                    Step::View(delta) => t.scroll_display(isize::from(delta)),
                    Step::Select(kind, r, c) => {
                        let kinds = [
                            crate::SelectionKind::Cell,
                            crate::SelectionKind::Word,
                            crate::SelectionKind::Line,
                            crate::SelectionKind::Block,
                        ];
                        let at = t.viewport_point(r, c);
                        t.select_start(kinds[usize::from(kind)], at);
                    }
                    Step::Extend(r, c) => t.select_extend(t.viewport_point(r, c)),
                    Step::Search(pattern) => {
                        t.search_set(pattern, false);
                    }
                    Step::Next(true) => {
                        t.search_next(None);
                    }
                    Step::Next(false) => {
                        t.search_prev(None);
                    }
                }
                now += Duration::from_millis(pause);
                let shown = paint_all(&frame);
                if !t.update_frame(&mut frame, now) {
                    prop_assert!(frame.damage().is_empty());
                    prop_assert_eq!(paint_all(&frame), shown, "a held picture never changes");
                    continue;
                }
                repaint(&mut canvas, &frame);
                let mut full = Frame::new();
                prop_assert!(t.update_frame(&mut full, now));
                prop_assert_eq!(&canvas, &paint_all(&full));
                prop_assert_eq!(&paint_all(&frame), &canvas);
                prop_assert_eq!(frame.cursor(), full.cursor());
                prop_assert_eq!(frame.placements(), full.placements());
                for c in frame.cells() {
                    let truth = t.grid().styles().get(StyleId(c.style));
                    prop_assert_eq!(frame.style(c.style), truth, "no stale style");
                }
                for r in 0..rows {
                    prop_assert_eq!(frame.stamp(r), full.stamp(r));
                }
            }
        }
    }
}
