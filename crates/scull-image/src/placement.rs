//! Where an image is shown: a cell rectangle anchored to an absolute line, a
//! source crop and a z-index. Kept apart from the image so one image can be
//! shown many times and a placement can move without touching pixels.

use crate::ImageId;

/// A source rectangle in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Crop {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width; zero means "to the right edge".
    pub width: u32,
    /// Height; zero means "to the bottom edge".
    pub height: u32,
}

impl Crop {
    /// The whole of a `width` x `height` image.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The part of this rectangle that lies inside a `width` x `height`
    /// image, with zero sizes widened to the image edge; kitty shows the
    /// intersection rather than rejecting an oversized crop.
    pub fn clamp(self, width: u32, height: u32) -> Self {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let fit = |want: u32, room: u32| if want == 0 { room } else { want.min(room) };
        Self {
            x,
            y,
            width: fit(self.width, width - x),
            height: fit(self.height, height - y),
        }
    }
}

/// One display of an image on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The image shown.
    pub image: ImageId,
    /// Kitty's placement id; zero when the client gave none.
    pub id: u32,
    /// Absolute line of the top-left cell. The terminal layer owns the
    /// numbering and re-anchors placements on scroll and reflow.
    pub row: u64,
    /// Column of the top-left cell.
    pub col: u32,
    /// Width in cells, at least one.
    pub cols: u32,
    /// Height in cells, at least one.
    pub rows: u32,
    /// Pixel offset inside the top-left cell.
    pub offset_x: u32,
    /// Pixel offset inside the top-left cell.
    pub offset_y: u32,
    /// Part of the image shown, already clamped to the image.
    pub crop: Crop,
    /// Stacking order; negative draws below text.
    pub z: i32,
}

impl Placement {
    /// Whether the cell at absolute line `row`, column `col` shows part of
    /// this placement.
    pub fn covers(&self, row: u64, col: u32) -> bool {
        self.covers_row(row) && self.covers_col(col)
    }

    /// Whether the placement spans absolute line `row`.
    pub fn covers_row(&self, row: u64) -> bool {
        row >= self.row && row - self.row < u64::from(self.rows)
    }

    /// Whether the placement spans column `col`.
    pub fn covers_col(&self, col: u32) -> bool {
        col >= self.col && col - self.col < self.cols
    }
}

/// Cells needed to show `pixels` with cells `cell` pixels long: at least one,
/// so a sliver of an image still owns a cell.
pub fn cells_for(pixels: u32, cell: u32) -> u32 {
    pixels.div_ceil(cell.max(1)).max(1)
}
