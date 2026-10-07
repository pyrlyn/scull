//! Terminal state: cursor, modes, charsets, margins and screens, driven by
//! the parser's actions and writing into the grid. Pure state with no I/O, so
//! conformance suites replay byte streams against it headlessly.
//!
//! Behaviour follows xterm's control sequence reference
//! (<https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>) and the DEC
//! manuals on vt100.net; where they differ, xterm wins because that is what
//! applications are tested against.

mod charset;
mod dispatch;
mod edit;
mod error;
mod modes;
mod motion;
mod pen;
mod print;
mod reply;
mod screen;
mod set_mode;
mod sgr;
mod state;
mod tabs;
mod terminal;

pub use error::TermError;
pub use modes::Modes;
pub use state::{Cursor, Margins};
pub use terminal::Terminal;
