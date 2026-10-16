//! Terminal state: cursor, modes, charsets, margins and screens, driven by
//! the parser's actions and writing into the grid. Pure state with no I/O, so
//! conformance suites replay byte streams against it headlessly.
//!
//! Behaviour follows xterm's control sequence reference
//! (<https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>) and the DEC
//! manuals on vt100.net; where they differ, xterm wins because that is what
//! applications are tested against.

mod charset;
mod damage;
mod dispatch;
mod edit;
mod error;
mod events;
mod frame;
mod images;
mod input;
mod modes;
mod motion;
mod osc;
mod pen;
mod preedit;
mod print;
mod reply;
mod resize;
mod screen;
mod search;
mod selection;
mod set_mode;
mod sgr;
mod state;
mod sync;
mod tabs;
mod terminal;
mod text;

pub use damage::{Damage, RowStamp, Scroll};
pub use error::TermError;
pub use events::{Clipboard, Event, TitleWhich};
pub use frame::{Frame, FrameCell, FrameCursor, FramePlacement, FrameRow, TextRun};
pub use images::DEFAULT_CELL_PX;
pub use modes::Modes;
pub use preedit::{FramePreedit, MAX_PREEDIT_BYTES};
pub use scull_image::{Crop, Image, ImageId, ImageStore, Placement};
pub use search::{MAX_SEARCH_MATCHES, MAX_SEARCH_PATTERN, Match};
pub use selection::{MAX_SELECTION_BYTES, MAX_WORD_CELLS, Point, SelectionKind, WORD_SEPARATORS};
pub use state::{Cursor, Margins};
pub use sync::{MAX_SYNC_BYTES, MAX_SYNC_HOLD};
pub use terminal::{MAX_CELL_PX, Terminal};
