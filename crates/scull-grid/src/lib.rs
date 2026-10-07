//! The cell grid: fixed-size cells, rows with stable ids, and the ring shared
//! by the screen and scrollback. Knows nothing about escape sequences so the
//! storage can be tested and measured without a parser.

mod cell;
mod cluster;
mod error;
mod grid;
mod intern;
mod reflow;
mod row;
mod style;

pub use cell::{CELL_BYTES, Cell, CellFlags, Content};
pub use cluster::{ClusterId, ClusterTable, MAX_CLUSTER_CHARS, MAX_CLUSTERS};
pub use error::GridError;
pub use grid::{Grid, MAX_RING_ROWS, Reflow, TrackPoint};
pub use intern::Marks;
pub use row::{LinkId, LinkSpan, Row, RowId};
pub use style::{Attrs, Color, MAX_STYLES, Style, StyleId, StyleTable, Underline};
