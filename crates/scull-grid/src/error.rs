//! The grid's error enum. Its own module so every storage module reports
//! failures in one vocabulary the terminal layer can match on.

use thiserror::Error;

/// Why a grid operation was refused. Every variant is recoverable: the
/// terminal layer degrades (default style, dropped marks) instead of failing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GridError {
    /// The style table holds its cap of live styles even after a sweep.
    #[error("style table full ({cap} live styles)")]
    StyleTableFull {
        /// The hard cap that was hit.
        cap: usize,
    },
    /// The cluster table holds its cap of live clusters even after a sweep.
    #[error("cluster table full ({cap} live clusters)")]
    ClusterTableFull {
        /// The hard cap that was hit.
        cap: usize,
    },
    /// A grapheme cluster longer than one cell may hold.
    #[error("cluster of {len} code points exceeds the cap of {max}")]
    ClusterTooLong {
        /// Code points in the rejected cluster.
        len: usize,
        /// The cap.
        max: usize,
    },
    /// A cluster of fewer than two code points; one code point lives inline.
    #[error("a cluster needs at least two code points")]
    ClusterTooShort,
    /// A column outside the row.
    #[error("column {col} outside a row of {cols} columns")]
    Column {
        /// The rejected column.
        col: u16,
        /// Row width.
        cols: u16,
    },
    /// A grid with no rows or no columns.
    #[error("a grid needs at least one row and one column")]
    Empty,
}
