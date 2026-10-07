//! Inline images on the grid: OSC 1337 `File=` (iTerm2), DCS `q` (sixel)
//! and APC `G` (kitty) are fed to `scull-image`'s decoders, the result is
//! placed at the cursor, and the cursor moves as each protocol says.
//! Separate from the dispatch table because images live beside the grid,
//! not in its cells.
//!
//! Placements are anchored to absolute lines: line `n` is logical grid row
//! `n - evicted`, where `evicted` counts the rows that ever left the top of
//! that screen's ring. Scrolling therefore moves nothing; only eviction
//! (history full, ED 3, the alternate screen's missing history) and
//! reflow touch placements. Cursor movement follows WezTerm's reading of
//! the protocols (`term/src/terminalstate/image.rs`, behaviour only, no
//! code): sixel leaves the cursor under the image's left edge, iTerm2 and
//! kitty right of its bottom-right cell. Leaving the alternate screen drops
//! its images, as foot does.

use scull_grid::TrackPoint;
use scull_image::{
    BYTES_PER_PIXEL, Crop, DEFAULT_QUOTA_BYTES, Dimension, Image, ImageStore, ItermArgs,
    ItermImage, KittyContext, KittyDecoder, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, Placement,
    SixelDecoder, cells_for,
};
use scull_parser::{Csi, End};

use crate::state::State;

/// Cell size assumed until the host reports one (`Terminal::set_cell_size`):
/// the classic 8x16 VGA text cell.
pub const DEFAULT_CELL_PX: (u32, u32) = (8, 16);

/// Background behind unpainted sixel pixels until the host sets its own:
/// opaque black.
pub(crate) const DEFAULT_BACKGROUND: [u8; BYTES_PER_PIXEL] = [0, 0, 0, u8::MAX];

/// The OSC number of iTerm2's proprietary commands.
const ITERM_OSC: &[u8] = b"1337;";

/// DCS `P2` of a sixel image whose unpainted pixels stay transparent.
const SIXEL_TRANSPARENT: u16 = 1;

/// Widest or tallest image in cells, whatever size a client asks for; the
/// cursor walks at most this many lines past an image.
const MAX_IMAGE_CELLS: u32 = MAX_IMAGE_SIDE;

const PERCENT: u64 = 100;

/// The images of one screen buffer and the absolute numbering of its lines.
#[derive(Debug, Clone)]
pub(crate) struct ScreenImages {
    pub(crate) store: ImageStore,
    kitty: KittyDecoder,
    /// Rows that left the top of this screen's grid: the absolute line of
    /// logical row 0.
    pub(crate) evicted: u64,
}

impl Default for ScreenImages {
    fn default() -> Self {
        Self {
            store: ImageStore::new(DEFAULT_QUOTA_BYTES),
            kitty: KittyDecoder::new(),
            evicted: 0,
        }
    }
}

impl ScreenImages {
    /// Drops every image and the kitty decoder's state; the numbering stays.
    pub(crate) fn clear(&mut self) {
        self.store.clear();
        self.kitty = KittyDecoder::new();
    }

    /// Drops the placements no line of the grid shows any more, and the
    /// sixel and iTerm2 images left without one (only kitty can place an
    /// image again).
    pub(crate) fn prune(&mut self) {
        let first = self.evicted;
        let gone = |p: &Placement| p.row.saturating_add(u64::from(p.rows)) <= first;
        if self.store.placements().iter().any(gone) {
            self.store.retain_placements(|p| !gone(p));
            self.store.prune_unplaced(|id| id.as_kitty().is_none());
        }
    }

    /// Drops the placements that touch absolute lines `lines`.
    pub(crate) fn remove_lines(&mut self, lines: std::ops::Range<u64>) {
        self.store.retain_placements(|p| {
            p.row >= lines.end || p.row.saturating_add(u64::from(p.rows)) <= lines.start
        });
        self.store.prune_unplaced(|id| id.as_kitty().is_none());
    }
}

/// Grows `image` with transparent pixels to whole cells, so a host that
/// scales the image to its cell rectangle shows it at its own size.
/// Kept as it is when the padded size would pass the caps.
fn pad_to_cells(image: Image, (cell_w, cell_h): (u32, u32)) -> Image {
    let (w, h) = (image.width(), image.height());
    let pad = |px: u32, cell: u32| cells_for(px, cell).saturating_mul(cell);
    let (pw, ph) = (pad(w, cell_w), pad(h, cell_h));
    let bytes = pw as usize * ph as usize * BYTES_PER_PIXEL;
    // Checked before allocating: the cell size comes from the host.
    let too_big = pw > MAX_IMAGE_SIDE || ph > MAX_IMAGE_SIDE || bytes > MAX_IMAGE_BYTES;
    if (pw, ph) == (w, h) || too_big {
        return image;
    }
    let row = image.stride();
    let mut pixels = vec![0; bytes];
    let padded_row = pw as usize * BYTES_PER_PIXEL;
    for (dst, src) in pixels
        .chunks_exact_mut(padded_row)
        .zip(image.pixels().chunks(row))
    {
        dst[..row].copy_from_slice(src);
    }
    Image::from_rgba(pw, ph, pixels).unwrap_or(image)
}

/// The pixel length an iTerm2 dimension asks for, `None` for `auto`.
fn requested(d: Dimension, cell: u32, screen_cells: u16) -> Option<u64> {
    match d {
        Dimension::Auto => None,
        Dimension::Cells(n) => Some(u64::from(n).saturating_mul(u64::from(cell))),
        Dimension::Pixels(n) => Some(u64::from(n)),
        Dimension::Percent(p) => Some(
            u64::from(screen_cells)
                .saturating_mul(u64::from(cell))
                .saturating_mul(u64::from(p))
                / PERCENT,
        ),
    }
}

impl State {
    /// The absolute line of screen row `r`.
    pub(crate) fn line_of(&self, r: u16) -> u64 {
        let history = u64::try_from(self.grid.history_len()).unwrap_or(u64::MAX);
        self.images
            .evicted
            .saturating_add(history)
            .saturating_add(u64::from(r))
    }

    /// Counts the rows a scroll of the whole screen by `lines` pushed out of
    /// the ring, given the history length before it.
    pub(crate) fn count_evicted(&mut self, lines: u16, history_before: usize) {
        let grown = self.grid.history_len().saturating_sub(history_before);
        let lost = usize::from(lines.min(self.rows())).saturating_sub(grown);
        let lost = u64::try_from(lost).unwrap_or(u64::MAX);
        self.images.evicted = self.images.evicted.saturating_add(lost);
    }

    /// DCS hook: a sixel image begins when the header is `P1;P2;P3 q`.
    pub(crate) fn sixel_start(&mut self, header: &Csi<'_>) {
        let sixel = header.final_byte == b'q'
            && header.intermediates.is_empty()
            && header.private.is_none();
        let p2 = header.params.iter().nth(1).and_then(|g| g.first().copied());
        let background = (p2 != Some(SIXEL_TRANSPARENT)).then_some(self.background);
        self.sixel = sixel.then(|| SixelDecoder::new(background));
    }

    pub(crate) fn sixel_put(&mut self, data: &[u8]) {
        if let Some(sixel) = &mut self.sixel {
            sixel.put(data);
        }
    }

    /// DCS end: a cut image still shows what arrived; a cancelled one
    /// shows nothing.
    pub(crate) fn sixel_end(&mut self, end: End) {
        let Some(sixel) = self.sixel.take() else {
            return;
        };
        if end == End::Cancelled {
            return;
        }
        let Ok(image) = sixel.finish() else {
            return;
        };
        let image = pad_to_cells(image, self.cell_px);
        let (cols, rows) = self.cells_of(&image);
        let col = self.cursor.col;
        if self.show(image, cols, rows) {
            self.advance_lines(rows);
            self.goto(self.cursor.row, col);
        }
    }

    /// OSC: iTerm2's `1337;File=` inline image; other OSCs are not handled.
    pub(crate) fn osc(&mut self, data: &[u8]) {
        let Some(payload) = data.strip_prefix(ITERM_OSC) else {
            return;
        };
        if !ItermImage::matches(payload) {
            return;
        }
        let Ok(ItermImage { image, args }) = ItermImage::decode(payload) else {
            return;
        };
        let auto = (args.width, args.height) == (Dimension::Auto, Dimension::Auto);
        let image = if auto {
            pad_to_cells(image, self.cell_px)
        } else {
            image
        };
        let (cols, rows) = self.iterm_cells(&args, &image);
        let col = self.cursor.col;
        if self.show(image, cols, rows) && args.move_cursor {
            self.move_past(col, cols, rows);
        }
    }

    pub(crate) fn kitty_start(&mut self) {
        self.images.kitty.start();
    }

    pub(crate) fn kitty_put(&mut self, data: &[u8]) {
        self.images.kitty.put(data);
    }

    /// APC end: runs the kitty command, queues its reply whole (or drops
    /// it when the reply queue is full) and moves the cursor.
    pub(crate) fn kitty_end(&mut self, end: End) {
        let ctx = KittyContext {
            row: self.line_of(self.cursor.row),
            col: u32::from(self.cursor.col),
            top: self.line_of(0),
            screen_rows: u32::from(self.rows()),
            cell_width: self.cell_px.0,
            cell_height: self.cell_px.1,
        };
        let images = &mut self.images;
        let outcome = images
            .kitty
            .finish(end == End::Complete, &mut images.store, &ctx);
        if let Some(reply) = outcome.reply {
            self.replies.push(&reply);
        }
        if let Some((cols, rows)) = outcome.cursor {
            self.move_past(self.cursor.col, cols, rows);
        }
    }

    /// Cells an image covers at its own pixel size.
    fn cells_of(&self, image: &Image) -> (u32, u32) {
        let (cell_w, cell_h) = self.cell_px;
        let cols = cells_for(image.width(), cell_w);
        let rows = cells_for(image.height(), cell_h);
        (cols.min(MAX_IMAGE_CELLS), rows.min(MAX_IMAGE_CELLS))
    }

    /// Cells for iTerm2's `width` and `height`; with both given and
    /// `preserveAspectRatio`, the image is fitted inside them.
    fn iterm_cells(&self, args: &ItermArgs, image: &Image) -> (u32, u32) {
        let (cell_w, cell_h) = self.cell_px;
        let (w, h) = (u64::from(image.width()), u64::from(image.height()));
        let want_w = requested(args.width, cell_w, self.cols());
        let want_h = requested(args.height, cell_h, self.rows());
        // `w` and `h` are at least 1 (images are never empty); products
        // saturate, since the requested sizes are the client's.
        let scale = |a: u64, num: u64, den: u64| a.saturating_mul(num) / den.max(1);
        let (w, h) = match (want_w, want_h) {
            (None, None) => (w, h),
            (Some(x), None) => (x, scale(h, x, w)),
            (None, Some(y)) => (scale(w, y, h), y),
            (Some(x), Some(y))
                if args.preserve_aspect && x.saturating_mul(h) <= y.saturating_mul(w) =>
            {
                (x, scale(h, x, w))
            }
            (Some(_), Some(y)) if args.preserve_aspect => (scale(w, y, h), y),
            (Some(x), Some(y)) => (x, y),
        };
        let cells = |px: u64, cell: u32| {
            let px = u32::try_from(px).unwrap_or(u32::MAX);
            cells_for(px, cell).min(MAX_IMAGE_CELLS)
        };
        (cells(w, cell_w), cells(h, cell_h))
    }

    /// Stores `image` and places it at the cursor, `cols` x `rows` cells.
    /// Earlier sixel and iTerm2 placements it covers whole are dropped,
    /// as foot does, so an animation drawn in place does not pile up.
    fn show(&mut self, image: Image, cols: u32, rows: u32) -> bool {
        let crop = Crop::full(image.width(), image.height());
        let (row, col) = (self.line_of(self.cursor.row), u32::from(self.cursor.col));
        let store = &mut self.images.store;
        let id = store.alloc_id();
        if store.insert(id, image).is_err() {
            return false;
        }
        let placement = Placement {
            image: id,
            id: 0,
            row,
            col,
            cols,
            rows,
            offset_x: 0,
            offset_y: 0,
            crop,
            z: 0,
        };
        let inside = |p: &Placement| {
            p.image.as_kitty().is_none()
                && p.z == placement.z
                && p.row >= row
                && p.col >= col
                && p.row.saturating_add(u64::from(p.rows)) <= row.saturating_add(u64::from(rows))
                && p.col.saturating_add(p.cols) <= col.saturating_add(cols)
        };
        store.retain_placements(|p| !inside(p));
        store.prune_unplaced(|old| old != id && old.as_kitty().is_none());
        store.place(placement).is_ok()
    }

    /// Moves down `lines` lines, scrolling at the bottom margin. Like any
    /// cursor move it ends the cluster being printed.
    fn advance_lines(&mut self, lines: u32) {
        self.end_cluster();
        for _ in 0..lines.min(MAX_IMAGE_CELLS) {
            self.index();
        }
    }

    /// Puts the cursor right of the bottom-right cell of an image placed at
    /// column `col`.
    fn move_past(&mut self, col: u16, cols: u32, rows: u32) {
        self.advance_lines(rows.saturating_sub(1));
        let right = u32::from(col).saturating_add(cols);
        self.goto(self.cursor.row, u16::try_from(right).unwrap_or(u16::MAX));
    }

    /// ED 2: images on the screen go with the text, as in kitty and foot.
    pub(crate) fn erase_screen_images(&mut self) {
        let lines = self.line_of(0)..self.line_of(self.rows());
        self.images.remove_lines(lines);
    }
}

/// Anchors of `images`' placements as tracking points for a resize, in
/// placement order. A placement whose top already left the grid is
/// anchored at logical row 0 and keeps its distance above it.
pub(crate) fn anchors(images: &ScreenImages) -> Vec<TrackPoint> {
    let first = images.evicted;
    let point = |p: &Placement| {
        let row = usize::try_from(p.row.saturating_sub(first)).unwrap_or(usize::MAX);
        TrackPoint::new(row, u16::try_from(p.col).unwrap_or(u16::MAX))
    };
    images.store.placements().iter().map(point).collect()
}

/// Re-anchors `images` after its grid was resized, from the `anchors`
/// the grid's reflow moved. A placement whose cell did not survive goes.
pub(crate) fn reanchor(images: &mut ScreenImages, anchors: &[TrackPoint]) {
    let first = images.evicted;
    let mut lost = Vec::with_capacity(anchors.len());
    for (p, a) in images.store.placements_mut().iter_mut().zip(anchors) {
        let above = first.saturating_sub(p.row);
        let row = u64::try_from(a.row).unwrap_or(u64::MAX);
        p.row = first.saturating_add(row).saturating_sub(above);
        p.col = u32::from(a.col);
        lost.push(a.clamped);
    }
    let mut at = 0;
    images.store.retain_placements(|_| {
        at += 1;
        !lost.get(at - 1).copied().unwrap_or(false)
    });
    images.store.prune_unplaced(|id| id.as_kitty().is_none());
}

#[cfg(test)]
mod tests;
