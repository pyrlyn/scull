//! The terminal's error enum. Its own module so the public constructor
//! reports failures in one vocabulary; once built, a terminal degrades
//! (default style, dropped cluster marks) instead of failing on PTY input.

use scull_grid::GridError;
use thiserror::Error;

/// Why a terminal could not be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TermError {
    /// The grid refused the requested size.
    #[error("grid: {0}")]
    Grid(#[from] GridError),
}
