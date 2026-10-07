//! VT parser: a bulk scanner that hands printable runs over in one piece,
//! in front of a table state machine for CSI, OSC, DCS and APC that emits
//! typed actions. Separate from the terminal state so it can be fuzzed and
//! benchmarked on its own.
//!
//! Everything it reads comes from the PTY and is untrusted: every buffer it
//! can grow has a hard cap ([`MAX_PARAMS`], [`MAX_INTERMEDIATES`],
//! [`MAX_OSC_BYTES`], [`MAX_DCS_BYTES`], [`MAX_APC_BYTES`]).

mod handler;
mod params;
mod parser;
mod scan;

pub use handler::{Csi, End, Esc, Handler, Osc};
pub use params::{MAX_PARAMS, Params};
pub use parser::{MAX_APC_BYTES, MAX_DCS_BYTES, MAX_INTERMEDIATES, MAX_OSC_BYTES, Parser};
