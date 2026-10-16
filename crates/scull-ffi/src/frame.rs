//! The frame handle: the UI's copy of the viewport (`scull_term::Frame`)
//! and the `repr(C)` view of it the host reads in place. Separate from the
//! terminal because a frame belongs to one UI thread and is read without
//! the terminal lock.
//!
//! Cells and text runs are handed out as the frame's own buffers: `tt_cell`
//! and `tt_run` have the layout of `FrameCell` and `TextRun`, checked at
//! compile time, so an update copies nothing for them. Rows, styles,
//! scrolls and image placements are rebuilt per update into small arrays
//! the frame owns.
//!
//! An image is a `tt_image`, the pointer of the core's `Arc<Image>`: the
//! frame holds one reference until its next update, and the host may take
//! its own with `tt_image_retain` to keep a texture's source past that.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::mem::offset_of;
use std::ptr;
use std::sync::Arc;
use std::time::Instant;

use scull_grid::{Attrs, Color, Style, Underline};
use scull_term::{Frame, FrameCell, FramePlacement, Image, MAX_PREEDIT_BYTES, TextRun};

use crate::guard::{SizedStruct, can_write, guard, tt_status, with_term, write_sized};
use crate::spawn::{text, tt_str};
use crate::term::tt_term;

/// One cell: 8 bytes, row after row, `cols * rows` of them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_cell {
    /// The first code point of the cell's text; 0 when it shows none.
    pub codepoint: u32,
    /// Index into `tt_frame_view.styles`.
    pub style: u16,
    /// Columns covered: 1, 2 for a wide character, 0 for the cell behind it.
    pub width: u8,
    /// `TT_CELL_CLUSTER`, `TT_CELL_SELECTED`, `TT_CELL_MATCH` and
    /// `TT_CELL_CURRENT_MATCH` bits.
    pub flags: u8,
}

/// The cell holds more code points than `codepoint`; the whole cluster is
/// in its row's text.
pub const TT_CELL_CLUSTER: u8 = 1;

/// The cell is selected.
pub const TT_CELL_SELECTED: u8 = 2;

/// The cell is part of a search match.
pub const TT_CELL_MATCH: u8 = 4;

/// The cell is part of the current search match (`TT_CELL_MATCH` is set too).
pub const TT_CELL_CURRENT_MATCH: u8 = 8;

/// Neighbouring cells of one style and width with their text, the unit the
/// platform shaper takes. Blank cells belong to no run.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_run {
    /// First column.
    pub col: u16,
    /// Columns covered, wide spacers included.
    pub cols: u16,
    /// Index into `tt_frame_view.styles`.
    pub style: u16,
    /// Columns per character: 1 or 2.
    pub width: u8,
    /// Byte offset of the run's text in its row's `text`.
    pub text_start: u32,
    /// Byte length of the run's text.
    pub text_len: u32,
}

const _: () = {
    assert!(size_of::<tt_cell>() == size_of::<FrameCell>());
    assert!(align_of::<tt_cell>() == align_of::<FrameCell>());
    assert!(offset_of!(tt_cell, codepoint) == offset_of!(FrameCell, codepoint));
    assert!(offset_of!(tt_cell, style) == offset_of!(FrameCell, style));
    assert!(offset_of!(tt_cell, width) == offset_of!(FrameCell, width));
    assert!(offset_of!(tt_cell, flags) == offset_of!(FrameCell, flags));
    assert!(TT_CELL_CLUSTER == FrameCell::CLUSTER);
    assert!(TT_CELL_SELECTED == FrameCell::SELECTED);
    assert!(TT_CELL_MATCH == FrameCell::MATCH);
    assert!(TT_CELL_CURRENT_MATCH == FrameCell::CURRENT_MATCH);
    assert!(size_of::<tt_run>() == size_of::<TextRun>());
    assert!(align_of::<tt_run>() == align_of::<TextRun>());
    assert!(offset_of!(tt_run, col) == offset_of!(TextRun, col));
    assert!(offset_of!(tt_run, cols) == offset_of!(TextRun, cols));
    assert!(offset_of!(tt_run, style) == offset_of!(TextRun, style));
    assert!(offset_of!(tt_run, width) == offset_of!(TextRun, width));
    assert!(offset_of!(tt_run, text_start) == offset_of!(TextRun, text_start));
    assert!(offset_of!(tt_run, text_len) == offset_of!(TextRun, text_len));
};

/// One viewport row: the line it shows and its text.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_row {
    /// The line shown; stays with the line as it scrolls, so it keys a
    /// cache of shaped rows.
    pub id: u64,
    /// Bumped by every change to the line.
    pub generation: u32,
    /// The row's text runs, left to right.
    pub runs: *const tt_run,
    /// Number of runs.
    pub runs_len: usize,
    /// UTF-8 text of all runs, not NUL-terminated.
    pub text: *const u8,
    /// Bytes of text.
    pub text_len: usize,
}

/// A colour: `TT_COLOR_DEFAULT`, `TT_COLOR_INDEXED | index` or
/// `TT_COLOR_RGB | 0xRRGGBB`; the kind is in `TT_COLOR_KIND_MASK`.
pub const TT_COLOR_DEFAULT: u32 = 0;
/// Palette colour, index in the low byte.
pub const TT_COLOR_INDEXED: u32 = 0x0100_0000;
/// True colour in the low three bytes.
pub const TT_COLOR_RGB: u32 = 0x0200_0000;
/// The bits that hold a colour's kind.
pub const TT_COLOR_KIND_MASK: u32 = 0xFF00_0000;

/// `tt_style.attrs` bits.
pub const TT_ATTR_BOLD: u16 = 1;
/// Faint.
pub const TT_ATTR_DIM: u16 = 0x02;
/// Italic.
pub const TT_ATTR_ITALIC: u16 = 0x04;
/// Blinking.
pub const TT_ATTR_BLINK: u16 = 0x08;
/// Foreground and background swapped.
pub const TT_ATTR_INVERSE: u16 = 0x10;
/// Invisible text.
pub const TT_ATTR_HIDDEN: u16 = 0x20;
/// Struck through.
pub const TT_ATTR_STRIKE: u16 = 0x40;
/// Line above.
pub const TT_ATTR_OVERLINE: u16 = 0x80;

const _: () = {
    assert!(TT_ATTR_BOLD == Attrs::BOLD.bits());
    assert!(TT_ATTR_DIM == Attrs::DIM.bits());
    assert!(TT_ATTR_ITALIC == Attrs::ITALIC.bits());
    assert!(TT_ATTR_BLINK == Attrs::BLINK.bits());
    assert!(TT_ATTR_INVERSE == Attrs::INVERSE.bits());
    assert!(TT_ATTR_HIDDEN == Attrs::HIDDEN.bits());
    assert!(TT_ATTR_STRIKE == Attrs::STRIKE.bits());
    assert!(TT_ATTR_OVERLINE == Attrs::OVERLINE.bits());
};

/// `tt_style.underline` values.
pub const TT_UNDERLINE_NONE: u8 = 0;
/// One line.
pub const TT_UNDERLINE_SINGLE: u8 = 1;
/// Two lines.
pub const TT_UNDERLINE_DOUBLE: u8 = 2;
/// Wavy.
pub const TT_UNDERLINE_CURLY: u8 = 3;
/// Dotted.
pub const TT_UNDERLINE_DOTTED: u8 = 4;
/// Dashed.
pub const TT_UNDERLINE_DASHED: u8 = 5;

/// A style as SGR set it; palette and reverse video are not resolved yet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_style {
    /// Foreground colour.
    pub fg: u32,
    /// Background colour.
    pub bg: u32,
    /// Underline colour; `TT_COLOR_DEFAULT` follows the foreground.
    pub underline_color: u32,
    /// `TT_ATTR_*` bits.
    pub attrs: u16,
    /// A `TT_UNDERLINE_*` value.
    pub underline: u8,
}

fn color(c: Color) -> u32 {
    match c {
        Color::Default => TT_COLOR_DEFAULT,
        Color::Indexed(i) => TT_COLOR_INDEXED | u32::from(i),
        Color::Rgb(r, g, b) => TT_COLOR_RGB | u32::from_be_bytes([0, r, g, b]),
    }
}

impl From<&Style> for tt_style {
    fn from(s: &Style) -> Self {
        Self {
            fg: color(s.fg),
            bg: color(s.bg),
            underline_color: color(s.underline_color),
            attrs: s.attrs.bits(),
            underline: match s.underline {
                Underline::None => TT_UNDERLINE_NONE,
                Underline::Single => TT_UNDERLINE_SINGLE,
                Underline::Double => TT_UNDERLINE_DOUBLE,
                Underline::Curly => TT_UNDERLINE_CURLY,
                Underline::Dotted => TT_UNDERLINE_DOTTED,
                Underline::Dashed => TT_UNDERLINE_DASHED,
            },
        }
    }
}

/// Rows `start..end` are, unchanged, the rows from `from` of the previous
/// picture: blit them before repainting the dirty rows.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_scroll {
    /// First row moved into.
    pub start: u16,
    /// One past the last row moved into.
    pub end: u16,
    /// Where `start` was in the previous picture.
    pub from: u16,
}

/// The cursor in viewport rows.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_cursor {
    /// Viewport row.
    pub row: u16,
    /// Column.
    pub col: u16,
    /// 0 when hidden or scrolled out of the viewport.
    pub visible: u8,
}

/// Where the frame shows the input method's composing text, set with
/// `tt_frame_preedit`. The text is already in the row's cells and runs and
/// the cursor is at its caret; a renderer only marks the span, usually
/// with an underline. `cols` is 0 when none is shown.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct tt_preedit {
    /// Viewport row.
    pub row: u16,
    /// First column.
    pub col: u16,
    /// Columns covered.
    pub cols: u16,
}

/// An image's pixels, shared and immutable. Opaque to the host: read it
/// with `tt_image_pixels`.
pub struct tt_image {
    // Never built: a `tt_image *` is an `Arc<Image>` pointer under its C name.
    _private: [u8; 0],
}

/// One image in the viewport. Draw the source rectangle of `image` scaled
/// to `cols` x `rows` cells whose top-left cell is (`row`, `col`), moved by
/// the pixel offset. Placements come lowest `z` first; a negative `z`
/// draws below the text.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_placement {
    /// The pixels; valid until the next update or free of the frame, or
    /// until `tt_image_release` after a `tt_image_retain`.
    pub image: *const tt_image,
    /// Changes whenever the pixels behind `image` may have; a texture cache
    /// keys on `image` and `generation`.
    pub generation: u64,
    /// Viewport row of the top-left cell; negative above the viewport.
    pub row: i32,
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
    /// Source rectangle in image pixels: left edge.
    pub src_x: u32,
    /// Top edge.
    pub src_y: u32,
    /// Width.
    pub src_w: u32,
    /// Height.
    pub src_h: u32,
    /// Stacking order.
    pub z: i32,
}

impl From<&FramePlacement> for tt_placement {
    fn from(p: &FramePlacement) -> Self {
        // A visible placement ends below row 0, so its row is above
        // `-rows`; the clamp only guards the conversion.
        let row = i32::try_from(p.row).unwrap_or(if p.row < 0 { i32::MIN } else { i32::MAX });
        Self {
            image: Arc::as_ptr(&p.image).cast::<tt_image>(),
            generation: p.image.generation(),
            row,
            col: p.col,
            cols: p.cols,
            rows: p.rows,
            offset_x: p.offset_x,
            offset_y: p.offset_y,
            src_x: p.crop.x,
            src_y: p.crop.y,
            src_w: p.crop.width,
            src_h: p.crop.height,
            z: p.z,
        }
    }
}

/// What `tt_frame_update` hands back. Set `struct_size` before the call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_frame_view {
    /// `sizeof(tt_frame_view)` as the host knows it; on return, the bytes
    /// the core filled in.
    pub struct_size: u32,
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
    /// 0 while synchronized output (mode 2026) holds the picture: nothing
    /// changed and there is no damage; keep showing the last frame.
    pub updated: u8,
    /// 1 when every row must be repainted (first frame, resize, other
    /// screen); every row is then also listed in `dirty`.
    pub full: u8,
    /// The cursor.
    pub cursor: tt_cursor,
    /// `cols * rows` cells, row-major.
    pub cells: *const tt_cell,
    /// Number of cells.
    pub cells_len: usize,
    /// One entry per viewport row.
    pub lines: *const tt_row,
    /// Number of rows in `lines`.
    pub lines_len: usize,
    /// The style table that cells and runs index; entry 0 is the default.
    pub styles: *const tt_style,
    /// Number of styles.
    pub styles_len: usize,
    /// Rows to blit from the previous picture, applied before `dirty`.
    pub scrolls: *const tt_scroll,
    /// Number of scrolls.
    pub scrolls_len: usize,
    /// Rows to repaint, top to bottom.
    pub dirty: *const u16,
    /// Number of dirty rows.
    pub dirty_len: usize,
    /// The images in the viewport, lowest `z` first. The rows they cover
    /// are in `dirty` whenever a placement over them changed.
    pub placements: *const tt_placement,
    /// Number of placements.
    pub placements_len: usize,
    /// The composing text's span; its row is in `dirty` whenever it
    /// changed.
    pub preedit: tt_preedit,
}

// SAFETY: repr(C), `struct_size` first, integers and raw pointers only.
unsafe impl SizedStruct for tt_frame_view {}

/// A UI-owned frame. Opaque to the host; one per view, used from one
/// thread at a time.
#[derive(Default)]
pub struct tt_frame {
    frame: Frame,
    lines: Vec<tt_row>,
    styles: Vec<tt_style>,
    scrolls: Vec<tt_scroll>,
    placements: Vec<tt_placement>,
    /// Set while an update runs; still set afterwards only if it panicked.
    torn: bool,
}

impl tt_frame {
    /// Rebuilds the arrays that point into `frame`, after it changed.
    fn rebuild(&mut self) {
        let frame = &self.frame;
        self.lines.clear();
        self.lines.extend((0..frame.rows()).map(|r| {
            let stamp = frame.stamp(r).unwrap_or_default();
            let (runs, text) = frame
                .row(r)
                .map_or((&[][..], ""), |row| (row.runs(), row.text()));
            tt_row {
                id: stamp.id.0,
                generation: stamp.generation,
                runs: runs.as_ptr().cast::<tt_run>(),
                runs_len: runs.len(),
                text: text.as_ptr(),
                text_len: text.len(),
            }
        }));
        self.styles.clear();
        self.styles.extend(
            (0..=u16::MAX)
                .map_while(|id| frame.style(id))
                .map(tt_style::from),
        );
        self.scrolls.clear();
        self.scrolls
            .extend(frame.damage().scrolls().iter().map(|s| tt_scroll {
                start: s.start,
                end: s.end,
                from: s.from,
            }));
        self.placements.clear();
        self.placements
            .extend(frame.placements().iter().map(tt_placement::from));
    }

    fn view(&self, updated: bool) -> tt_frame_view {
        let frame = &self.frame;
        let cursor = frame.cursor();
        let cells = frame.cells();
        let dirty = frame.damage().dirty();
        tt_frame_view {
            struct_size: 0,
            cols: frame.cols(),
            rows: frame.rows(),
            updated: u8::from(updated),
            full: u8::from(frame.damage().is_full()),
            cursor: tt_cursor {
                row: cursor.map_or(0, |c| c.row),
                col: cursor.map_or(0, |c| c.col),
                visible: u8::from(cursor.is_some()),
            },
            cells: cells.as_ptr().cast::<tt_cell>(),
            cells_len: cells.len(),
            lines: self.lines.as_ptr(),
            lines_len: self.lines.len(),
            styles: self.styles.as_ptr(),
            styles_len: self.styles.len(),
            scrolls: self.scrolls.as_ptr(),
            scrolls_len: self.scrolls.len(),
            dirty: dirty.as_ptr(),
            dirty_len: dirty.len(),
            placements: self.placements.as_ptr(),
            placements_len: self.placements.len(),
            preedit: frame
                .preedit()
                .map_or_else(tt_preedit::default, |p| tt_preedit {
                    row: p.row,
                    col: p.col,
                    cols: p.cols,
                }),
        }
    }
}

/// A new, empty frame; the first update fills it. `NULL` only if the core
/// failed. Free it with `tt_frame_free`.
#[unsafe(no_mangle)]
pub extern "C" fn tt_frame_new() -> *mut tt_frame {
    let mut out = ptr::null_mut();
    let _ = guard(|| {
        out = Box::into_raw(Box::<tt_frame>::default());
        tt_status::TT_OK
    });
    out
}

/// Brings `frame` up to date with `term`'s viewport and describes it in
/// `*view`. Holds the terminal lock only while it copies the rows that
/// changed. The pointers in `*view` stay valid until the next update or
/// free of this frame.
///
/// # Safety
///
/// `frame` and `term` are `NULL` or live; `view` is `NULL` or points to
/// `struct_size` writable bytes. No other thread uses `frame` meanwhile.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_frame_update(
    frame: *mut tt_frame,
    term: *const tt_term,
    view: *mut tt_frame_view,
) -> tt_status {
    let body = |t: &tt_term| {
        // SAFETY: NULL or live and unshared, by the caller's contract.
        let Some(frame) = (unsafe { frame.as_mut() }) else {
            return tt_status::TT_INVALID;
        };
        // Checked first: damage taken by an update must reach the host.
        // SAFETY: the caller's contract.
        if !unsafe { can_write(view) } {
            return tt_status::TT_INVALID;
        }
        // An update that panicked left the frame half-written: start over.
        if frame.torn {
            *frame = tt_frame::default();
        }
        frame.torn = true;
        t.rearm_wakeup();
        let updated = t
            .core()
            .lock()
            .term
            .update_frame(&mut frame.frame, Instant::now());
        frame.rebuild();
        frame.torn = false;
        // SAFETY: checked writable above.
        unsafe { write_sized(view, &frame.view(updated)) };
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Sets the input method's composing text, UTF-8, with the caret at byte
/// `caret` (moved back onto a character boundary); `len` 0 clears it. The
/// next `tt_frame_update` shows it at the cursor (`tt_frame_view.preedit`).
/// It never reaches the child: commit text with `tt_term_text`. Text past
/// `TT_MAX_PREEDIT_BYTES` is cut. `TT_INVALID` for a `NULL` frame or text
/// that is not UTF-8.
///
/// # Safety
///
/// `frame` is `NULL` or live and not used by another thread meanwhile;
/// `bytes` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_frame_preedit(
    frame: *mut tt_frame,
    bytes: *const u8,
    len: usize,
    caret: usize,
) -> tt_status {
    guard(|| {
        // SAFETY: NULL or live and unshared, by the caller's contract.
        let Some(frame) = (unsafe { frame.as_mut() }) else {
            return tt_status::TT_INVALID;
        };
        // SAFETY: the caller's contract.
        let Some(text) = (unsafe { text(tt_str { ptr: bytes, len }) }) else {
            return tt_status::TT_INVALID;
        };
        frame.frame.set_preedit(text, caret);
        tt_status::TT_OK
    })
}

/// Most bytes of composing text a frame shows.
pub const TT_MAX_PREEDIT_BYTES: usize = 1024;

const _: () = assert!(TT_MAX_PREEDIT_BYTES == MAX_PREEDIT_BYTES);

/// Frees the frame and every buffer its views pointed to. `NULL` is a
/// no-op.
///
/// # Safety
///
/// `frame` is `NULL` or live, and not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_frame_free(frame: *mut tt_frame) {
    if frame.is_null() {
        return;
    }
    let _ = guard(|| {
        // SAFETY: from `Box::into_raw` in `tt_frame_new`, freed only here.
        drop(unsafe { Box::from_raw(frame) });
        tt_status::TT_OK
    });
}

/// Takes a reference to `image`, so its pixels outlive the frame update
/// that handed it out. `NULL` is a no-op.
///
/// # Safety
///
/// `image` is `NULL`, a `tt_placement.image` whose view is still valid, or
/// an image retained and not yet released. Any thread may call this.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_image_retain(image: *const tt_image) {
    if image.is_null() {
        return;
    }
    let _ = guard(|| {
        // SAFETY: the pointer of a live `Arc<Image>`, by the caller's
        // contract; the count it adds is dropped by `tt_image_release`.
        unsafe { Arc::increment_strong_count(image.cast::<Image>()) };
        tt_status::TT_OK
    });
}

/// Drops a reference taken by `tt_image_retain`. `NULL` is a no-op.
///
/// # Safety
///
/// `image` is `NULL` or was retained, and each retain is released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_image_release(image: *const tt_image) {
    if image.is_null() {
        return;
    }
    let _ = guard(|| {
        // SAFETY: the caller's contract: this releases its own retain, so
        // the count stays positive for every other holder.
        unsafe { Arc::decrement_strong_count(image.cast::<Image>()) };
        tt_status::TT_OK
    });
}

/// The pixels of `image`: `height` rows of `stride` bytes, each `width`
/// RGBA pixels of 4 bytes, straight (not premultiplied) alpha. Valid while
/// `image` is. Each out pointer may be `NULL`; a `NULL` image answers
/// `NULL` and zeroes them.
///
/// # Safety
///
/// `image` is as for `tt_image_retain`; each out pointer is `NULL` or
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_image_pixels(
    image: *const tt_image,
    width: *mut u32,
    height: *mut u32,
    stride: *mut usize,
) -> *const u8 {
    let mut out = ptr::null();
    let _ = guard(|| {
        // SAFETY: NULL or a live `Image`, by the caller's contract.
        let image = unsafe { image.cast::<Image>().as_ref() };
        let (w, h, s) = image.map_or((0, 0, 0), |i| (i.width(), i.height(), i.stride()));
        out = image.map_or(ptr::null(), |i| i.pixels().as_ptr());
        // SAFETY: each is NULL or writable, by the caller's contract.
        unsafe {
            if let Some(width) = width.as_mut() {
                *width = w;
            }
            if let Some(height) = height.as_mut() {
                *height = h;
            }
            if let Some(stride) = stride.as_mut() {
                *stride = s;
            }
        }
        tt_status::TT_OK
    });
    out
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    fn blank_view() -> tt_frame_view {
        // SAFETY: `tt_frame_view` is `SizedStruct`: all-zero is valid.
        let mut view: tt_frame_view = unsafe { std::mem::zeroed() };
        view.struct_size = u32::try_from(size_of::<tt_frame_view>()).unwrap();
        view
    }

    fn update(frame: *mut tt_frame, term: *const tt_term) -> (tt_status, tt_frame_view) {
        let mut view = blank_view();
        // SAFETY: live handles and a whole view.
        let status = unsafe { tt_frame_update(frame, term, &mut view) };
        (status, view)
    }

    /// The slice behind a view pointer.
    fn parts<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
        // SAFETY: the view's pointers are valid until the next update, and
        // every test reads them before that.
        unsafe { slice::from_raw_parts(ptr, len) }
    }

    fn row_text(view: &tt_frame_view, r: usize) -> &str {
        let row = parts(view.lines, view.lines_len)[r];
        std::str::from_utf8(parts(row.text, row.text_len)).unwrap()
    }

    fn with_frame(cols: u16, rows: u16, test: impl FnOnce(*mut tt_frame, *mut tt_term)) {
        let (frame, term) = (tt_frame_new(), new_term(cols, rows));
        test(frame, term);
        // SAFETY: live handles, not used again.
        unsafe {
            tt_frame_free(frame);
            tt_term_free(term);
        }
    }

    #[test]
    fn the_first_update_describes_the_whole_screen() {
        with_frame(6, 3, |frame, term| {
            feed(term, "ab\u{4e2d}\r\n\x1b[1mx".as_bytes());
            let (status, view) = update(frame, term);
            assert_eq!(status, tt_status::TT_OK);
            assert_eq!(
                (view.cols, view.rows, view.updated, view.full),
                (6, 3, 1, 1)
            );
            assert_eq!(view.cells_len, 18);
            assert_eq!(parts(view.dirty, view.dirty_len), [0, 1, 2]);
            let cells = parts(view.cells, view.cells_len);
            assert_eq!(cells[2].codepoint, 0x4e2d);
            assert_eq!((cells[2].width, cells[3].width), (2, 0));
            assert_eq!(
                (row_text(&view, 0), row_text(&view, 1)),
                ("ab\u{4e2d}", "x")
            );
            let runs = parts(parts(view.lines, view.lines_len)[0].runs, 2);
            assert_eq!((runs[0].cols, runs[1].col, runs[1].width), (2, 2, 2));
            let styles = parts(view.styles, view.styles_len);
            assert_eq!(styles[0], tt_style::default());
            let bold = cells[6].style;
            assert_eq!(styles[usize::from(bold)].attrs, TT_ATTR_BOLD);
            assert_eq!(
                view.cursor,
                tt_cursor {
                    row: 1,
                    col: 1,
                    visible: 1
                }
            );
        });
    }

    #[test]
    fn later_updates_report_only_the_damage() {
        with_frame(4, 3, |frame, term| {
            update(frame, term);
            feed(term, b"\x1b[3Hz");
            let (_, view) = update(frame, term);
            assert_eq!(
                (view.full, parts(view.dirty, view.dirty_len)),
                (0, &[2][..])
            );
            feed(term, b"\r\n");
            let (_, view) = update(frame, term);
            let scroll = tt_scroll {
                start: 0,
                end: 2,
                from: 1,
            };
            assert_eq!(parts(view.scrolls, view.scrolls_len), [scroll]);
            assert_eq!(row_text(&view, 1), "z");
            let (_, view) = update(frame, term);
            assert_eq!((view.dirty_len, view.scrolls_len), (0, 0));
        });
    }

    #[test]
    fn synchronized_output_holds_the_last_picture() {
        with_frame(4, 2, |frame, term| {
            update(frame, term);
            feed(term, b"\x1b[?2026hab");
            let (status, view) = update(frame, term);
            assert_eq!(
                (status, view.updated, view.dirty_len),
                (tt_status::TT_OK, 0, 0)
            );
            assert_eq!(row_text(&view, 0), "");
            feed(term, b"\x1b[?2026l");
            let (_, view) = update(frame, term);
            assert_eq!((view.updated, row_text(&view, 0)), (1, "ab"));
        });
    }

    #[test]
    fn styles_carry_colours_attributes_and_underlines() {
        let style = Style {
            fg: Color::Rgb(1, 2, 3),
            bg: Color::Indexed(9),
            underline: Underline::Curly,
            attrs: Attrs::ITALIC | Attrs::STRIKE,
            ..Style::default()
        };
        let c = tt_style::from(&style);
        assert_eq!(
            (c.fg, c.bg, c.underline_color),
            (TT_COLOR_RGB | 0x01_0203, TT_COLOR_INDEXED | 9, 0)
        );
        assert_eq!(c.fg & TT_COLOR_KIND_MASK, TT_COLOR_RGB);
        assert_eq!(
            (c.attrs, c.underline),
            (TT_ATTR_ITALIC | TT_ATTR_STRIKE, TT_UNDERLINE_CURLY)
        );
    }

    #[test]
    fn a_short_view_gets_its_prefix_and_a_bad_one_keeps_the_damage() {
        with_frame(4, 2, |frame, term| {
            let mut bad = blank_view();
            bad.struct_size = 2;
            // SAFETY: a whole view.
            let status = unsafe { tt_frame_update(frame, term, &mut bad) };
            assert_eq!(status, tt_status::TT_INVALID);
            let mut short = blank_view();
            let prefix = u32::try_from(offset_of!(tt_frame_view, cursor)).unwrap();
            short.struct_size = prefix;
            // SAFETY: a whole view.
            let status = unsafe { tt_frame_update(frame, term, &mut short) };
            assert_eq!(status, tt_status::TT_OK);
            assert_eq!((short.cols, short.full, short.struct_size), (4, 1, prefix));
            assert!(short.cells.is_null(), "past struct_size nothing is written");
        });
    }

    #[test]
    fn null_handles_and_poisoned_terminals_are_refused() {
        with_frame(4, 2, |frame, term| {
            let mut view = blank_view();
            // SAFETY: NULL is allowed for every argument.
            unsafe {
                assert_eq!(
                    tt_frame_update(ptr::null_mut(), term, &mut view),
                    tt_status::TT_INVALID
                );
                assert_eq!(
                    tt_frame_update(frame, ptr::null(), &mut view),
                    tt_status::TT_INVALID
                );
                assert_eq!(
                    tt_frame_update(frame, term, ptr::null_mut()),
                    tt_status::TT_INVALID
                );
                (*term).poison();
            }
            assert_eq!(update(frame, term).0, tt_status::TT_POISONED);
        });
        // SAFETY: NULL is a no-op.
        unsafe { tt_frame_free(ptr::null_mut()) };
    }

    /// kitty: image 1, 2 x 2 RGB pixels, shown at the cursor without a reply.
    const KITTY: &[u8] = b"\x1b_Gi=1,f=24,s=2,v=2,a=T,q=2;AAAAAAAAAAAAAAAA\x1b\\";

    fn pixels_of(image: *const tt_image) -> (Option<Vec<u8>>, u32, u32, usize) {
        let (mut w, mut h, mut stride) = (u32::MAX, u32::MAX, usize::MAX);
        // SAFETY: a live image or NULL, and writable outs.
        let p = unsafe { tt_image_pixels(image, &mut w, &mut h, &mut stride) };
        let bytes = (!p.is_null()).then(|| parts(p, stride * h as usize).to_vec());
        (bytes, w, h, stride)
    }

    #[test]
    fn placements_hand_out_images_that_a_retain_keeps_alive() {
        let (frame, term) = (tt_frame_new(), new_term(6, 3));
        feed(term, b"\x1b[2;3H");
        feed(term, KITTY);
        let (_, view) = update(frame, term);
        let [p] = parts(view.placements, view.placements_len) else {
            panic!("one placement");
        };
        assert_eq!((p.row, p.col, p.cols, p.rows, p.z), (1, 2, 1, 1, 0));
        assert_eq!((p.src_x, p.src_y, p.src_w, p.src_h), (0, 0, 2, 2));
        assert_eq!(parts(view.dirty, view.dirty_len), [0, 1, 2]);
        let image = p.image;
        // SAFETY: from a valid view.
        unsafe { tt_image_retain(image) };
        // SAFETY: live handles, not used again.
        unsafe {
            tt_frame_free(frame);
            tt_term_free(term);
        }
        assert_eq!(pixels_of(image), (Some([0, 0, 0, 255].repeat(4)), 2, 2, 8));
        // SAFETY: retained above, released once.
        unsafe { tt_image_release(image) };
    }

    #[test]
    fn a_null_image_has_no_pixels() {
        assert_eq!(pixels_of(ptr::null()), (None, 0, 0, 0));
        // SAFETY: NULL is allowed everywhere.
        unsafe {
            tt_image_retain(ptr::null());
            tt_image_release(ptr::null());
            let _ = tt_image_pixels(
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
    }

    #[test]
    fn an_image_damages_its_rows_and_an_old_view_lacks_placements() {
        with_frame(6, 3, |frame, term| {
            update(frame, term);
            feed(term, b"\x1b[2;1H");
            feed(term, KITTY);
            let mut old = blank_view();
            old.struct_size = u32::try_from(offset_of!(tt_frame_view, placements)).unwrap();
            // SAFETY: a whole view.
            let status = unsafe { tt_frame_update(frame, term, &mut old) };
            assert_eq!(status, tt_status::TT_OK);
            assert_eq!(parts(old.dirty, old.dirty_len), [1]);
            assert!(old.placements.is_null(), "a host built before images");
        });
    }

    #[test]
    fn a_resize_with_pixels_sets_the_cell_size_images_use() {
        with_frame(10, 4, |frame, term| {
            // SAFETY: a live handle.
            let status = unsafe { crate::term::tt_term_resize(term, 10, 4, 40, 80) };
            assert_eq!(status, tt_status::TT_OK);
            feed(
                term,
                b"\x1b]1337;File=inline=1;width=8px;height=40px;preserveAspectRatio=0:\
                  iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\x07",
            );
            let (_, view) = update(frame, term);
            let p = parts(view.placements, view.placements_len)[0];
            assert_eq!((p.cols, p.rows), (2, 2), "cells of 4 x 20 px");
        });
    }

    fn set_preedit(frame: *mut tt_frame, text: &[u8], caret: usize) -> tt_status {
        // SAFETY: a live frame and `text.len()` readable bytes.
        unsafe { tt_frame_preedit(frame, text.as_ptr(), text.len(), caret) }
    }

    #[test]
    fn a_preedit_is_drawn_at_the_cursor_until_it_is_cleared() {
        with_frame(8, 2, |frame, term| {
            feed(term, b"ab");
            update(frame, term);
            let text = "日本".as_bytes();
            assert_eq!(set_preedit(frame, text, text.len()), tt_status::TT_OK);
            let (_, view) = update(frame, term);
            let span = tt_preedit {
                row: 0,
                col: 2,
                cols: 4,
            };
            assert_eq!(view.preedit, span);
            assert_eq!(row_text(&view, 0), "ab日本");
            assert_eq!(parts(view.dirty, view.dirty_len), [0]);
            assert_eq!((view.cursor.row, view.cursor.col), (0, 6));
            // SAFETY: a live frame; `NULL` with length 0 is empty text.
            let status = unsafe { tt_frame_preedit(frame, ptr::null(), 0, 0) };
            assert_eq!(status, tt_status::TT_OK);
            let (_, view) = update(frame, term);
            assert_eq!(
                (view.preedit, row_text(&view, 0)),
                (tt_preedit::default(), "ab")
            );
        });
    }

    #[test]
    fn a_bad_preedit_is_refused_and_an_old_view_lacks_it() {
        with_frame(8, 2, |frame, term| {
            assert_eq!(set_preedit(frame, b"\xff", 0), tt_status::TT_INVALID);
            assert_eq!(set_preedit(ptr::null_mut(), b"x", 0), tt_status::TT_INVALID);
            // SAFETY: NULL bytes with a length are refused before any read.
            let status = unsafe { tt_frame_preedit(frame, ptr::null(), 1, 0) };
            assert_eq!(status, tt_status::TT_INVALID);
            assert_eq!(set_preedit(frame, b"x", 0), tt_status::TT_OK);
            let mut old = blank_view();
            old.struct_size = u32::try_from(offset_of!(tt_frame_view, preedit)).unwrap();
            old.preedit.cols = 9;
            // SAFETY: a whole view.
            let status = unsafe { tt_frame_update(frame, term, &mut old) };
            assert_eq!((status, old.preedit.cols), (tt_status::TT_OK, 9));
        });
    }

    #[test]
    fn a_torn_frame_is_repainted_whole() {
        with_frame(4, 2, |frame, term| {
            update(frame, term);
            // SAFETY: live and unshared.
            unsafe { (*frame).torn = true };
            let (_, view) = update(frame, term);
            assert_eq!((view.full, view.dirty_len), (1, 2));
        });
    }
}
