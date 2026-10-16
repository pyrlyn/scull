//! The state the parser's actions drive: grid, cursor, pen, margins, tab
//! stops and modes. Separate from `terminal.rs` so the parser can borrow it
//! as its handler while the public type owns both.

use scull_grid::{Cell, Grid, Row};
use scull_image::{BYTES_PER_PIXEL, SixelDecoder};
use scull_unicode::{ClusterPolicy, GraphemeState, WidthOptions};

use crate::charset::Charsets;
use crate::events::Queue;
use crate::images::{DEFAULT_BACKGROUND, DEFAULT_CELL_PX, ScreenImages};
use crate::modes::Modes;
use crate::osc::{Links, Pending};
use crate::pen::Pen;
use crate::reply::Replies;
use crate::screen::SavedCursor;
use crate::tabs::TabStops;

/// Where the next character goes. Rows and columns count from 0 at the top
/// left of the screen, whatever the origin mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Screen row.
    pub row: u16,
    /// Column.
    pub col: u16,
    /// The last column was just written with autowrap on: the next printable
    /// character wraps first (DEC STD 070 "last column flag").
    pub pending_wrap: bool,
}

/// Scroll margins, inclusive, counted from 0: DECSTBM sets `top` and
/// `bottom`, DECSLRM sets `left` and `right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margins {
    /// First row of the scroll region.
    pub top: u16,
    /// Last row of the scroll region.
    pub bottom: u16,
    /// First column of the scroll region.
    pub left: u16,
    /// Last column of the scroll region.
    pub right: u16,
}

impl Margins {
    /// The whole screen.
    pub(crate) fn full(cols: u16, rows: u16) -> Self {
        Self {
            top: 0,
            bottom: rows.saturating_sub(1),
            left: 0,
            right: cols.saturating_sub(1),
        }
    }
}

/// Everything the handler mutates.
#[derive(Debug, Clone)]
pub(crate) struct State {
    /// The screen shown.
    pub(crate) grid: Grid,
    /// The screen not shown: the alternate one, or the main one while the
    /// alternate is active.
    pub(crate) alt: Grid,
    pub(crate) alt_active: bool,
    /// DECSC state for the main and the alternate screen.
    pub(crate) saved: [Option<SavedCursor>; 2],
    pub(crate) charsets: Charsets,
    pub(crate) replies: Replies,
    pub(crate) cursor: Cursor,
    pub(crate) pen: Pen,
    pub(crate) modes: Modes,
    pub(crate) margins: Margins,
    pub(crate) tabs: TabStops,
    pub(crate) width: WidthOptions,
    /// Segmentation of the text printed since the last other action.
    pub(crate) grapheme: GraphemeState,
    /// The last character printed, for REP.
    pub(crate) last_char: Option<char>,
    /// Bell, clipboard and links. Titles and the directory are events only.
    pub(crate) events: Queue,
    pub(crate) links: Links,
    /// OSC 52 reads the host has not answered.
    pub(crate) clips: Vec<Pending>,
    /// Images of the screen shown, and of the other one; they switch with
    /// the grids.
    pub(crate) images: ScreenImages,
    pub(crate) alt_images: ScreenImages,
    /// The sixel image a DCS is streaming, if any.
    pub(crate) sixel: Option<SixelDecoder>,
    /// Width and height of a cell in pixels, as the host last reported.
    pub(crate) cell_px: (u32, u32),
    /// What unpainted sixel pixels show unless the image asks for
    /// transparency.
    pub(crate) background: [u8; BYTES_PER_PIXEL],
}

impl State {
    /// `alt` must have the size of `grid`.
    pub(crate) fn new(grid: Grid, alt: Grid, width: WidthOptions) -> Self {
        let (cols, rows) = (grid.cols(), grid.screen_rows());
        Self {
            grid,
            alt,
            alt_active: false,
            saved: [None, None],
            charsets: Charsets::default(),
            replies: Replies::default(),
            cursor: Cursor::default(),
            pen: Pen::default(),
            modes: Modes::default(),
            margins: Margins::full(cols, rows),
            tabs: TabStops::new(cols),
            width,
            grapheme: GraphemeState::new(width.version),
            last_char: None,
            events: Queue::default(),
            links: Links::default(),
            clips: Vec::new(),
            images: ScreenImages::default(),
            alt_images: ScreenImages::default(),
            sixel: None,
            cell_px: DEFAULT_CELL_PX,
            background: DEFAULT_BACKGROUND,
        }
    }

    pub(crate) fn cols(&self) -> u16 {
        self.grid.cols()
    }

    pub(crate) fn rows(&self) -> u16 {
        self.grid.screen_rows()
    }

    pub(crate) fn last_col(&self) -> u16 {
        self.cols().saturating_sub(1)
    }

    pub(crate) fn last_row(&self) -> u16 {
        self.rows().saturating_sub(1)
    }

    /// The cursor's row for writing.
    pub(crate) fn cursor_row(&mut self) -> Option<&mut Row> {
        self.grid.screen_row_mut(self.cursor.row)
    }

    /// What erasing leaves behind with the current pen.
    pub(crate) fn blank(&mut self) -> Cell {
        self.pen.blank(&mut self.grid)
    }

    /// Any action other than printing ends the cluster being built, so a
    /// combining mark after a cursor move never reaches back across it.
    pub(crate) fn end_cluster(&mut self) {
        self.grapheme = GraphemeState::new(self.width.version);
    }

    pub(crate) fn policy(&self) -> ClusterPolicy {
        if self.modes.grapheme_clusters {
            ClusterPolicy::Grapheme
        } else {
            ClusterPolicy::FirstCodepoint
        }
    }

    /// The column printing stops at: the right margin, or the screen edge
    /// for a cursor already right of the margin (xterm).
    pub(crate) fn line_end(&self) -> u16 {
        if self.cursor.col <= self.margins.right {
            self.margins.right
        } else {
            self.last_col()
        }
    }

    /// Whether the cursor is inside the left and right margins, where
    /// scrolling and line edits act.
    pub(crate) fn in_lr_margins(&self) -> bool {
        (self.margins.left..=self.margins.right).contains(&self.cursor.col)
    }

    /// Whether the cursor is inside the top and bottom margins.
    pub(crate) fn in_tb_margins(&self) -> bool {
        (self.margins.top..=self.margins.bottom).contains(&self.cursor.row)
    }
}
