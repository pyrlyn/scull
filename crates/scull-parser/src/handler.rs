//! The typed actions the parser emits and the [`Handler`] trait that receives
//! them. Separate from the state machine so the terminal state (scull-term)
//! depends on this vocabulary, not on how the bytes were scanned.
//!
//! Every action borrows from the parser's reusable buffers or from the input
//! slice itself, so delivering one allocates nothing.

use crate::params::Params;

/// An escape sequence: `ESC`, intermediates, final byte.
///
/// The seven-bit form of a C1 control is delivered here too: `U+0085` (NEL)
/// arrives as `ESC E`, the way ECMA-48 defines their equivalence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Esc<'a> {
    /// Bytes `0x20..=0x2F` between `ESC` and the final byte.
    pub intermediates: &'a [u8],
    /// The byte that ended the sequence, `0x30..=0x7E`.
    pub final_byte: u8,
}

/// A control sequence (CSI), or the header of a device control string (DCS),
/// which has the same shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Csi<'a> {
    /// Numeric parameters, colon sub-parameters grouped with their parameter.
    pub params: &'a Params,
    /// Bytes `0x20..=0x2F` before the final byte.
    pub intermediates: &'a [u8],
    /// A leading `<`, `=`, `>` or `?` that marks a private sequence.
    pub private: Option<u8>,
    /// The byte that ended the sequence, `0x40..=0x7E`.
    pub final_byte: u8,
    /// More parameters arrived than [`crate::MAX_PARAMS`]; the excess was dropped.
    pub truncated: bool,
}

/// An operating system command (OSC) payload, terminated by BEL or ST.
///
/// C0 controls inside the payload are dropped before it is stored, so a title
/// or a link can never carry one through to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Osc<'a> {
    /// Everything between `ESC ]` and the terminator.
    pub data: &'a [u8],
    /// Ended by BEL rather than ST; a reply should use the same terminator.
    pub bell_terminated: bool,
}

impl<'a> Osc<'a> {
    /// The payload split on `;`. Splitting is lazy so the parser keeps no
    /// index table and needs no cap on the number of fields.
    pub fn params(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        self.data.split(|&b| b == b';')
    }
}

/// How a streamed string (DCS or APC) ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Terminated by ST; every byte was delivered.
    Complete,
    /// Terminated by ST, but bytes past the cap were dropped.
    Truncated,
    /// Aborted by CAN, SUB or a new escape; the payload must be discarded.
    Cancelled,
}

/// Receives the parser's actions. Every method defaults to doing nothing, so
/// a consumer implements only what it understands.
pub trait Handler {
    /// A run of printable text: everything up to the next control byte, with
    /// invalid UTF-8 replaced by U+FFFD.
    fn print(&mut self, _text: &str) {}
    /// A C0 control. DEL is ignored and never arrives here.
    fn execute(&mut self, _byte: u8) {}
    /// An escape sequence.
    fn esc_dispatch(&mut self, _esc: &Esc<'_>) {}
    /// A control sequence.
    fn csi_dispatch(&mut self, _csi: &Csi<'_>) {}
    /// An operating system command.
    fn osc_dispatch(&mut self, _osc: &Osc<'_>) {}
    /// A device control string begins; its payload follows through `dcs_put`.
    fn dcs_hook(&mut self, _header: &Csi<'_>) {}
    /// A slice of DCS payload, borrowed straight from the input.
    fn dcs_put(&mut self, _data: &[u8]) {}
    /// The device control string ended.
    fn dcs_unhook(&mut self, _end: End) {}
    /// An application program command (APC) begins.
    fn apc_start(&mut self) {}
    /// A slice of APC payload, borrowed straight from the input.
    fn apc_put(&mut self, _data: &[u8]) {}
    /// The application program command ended.
    fn apc_end(&mut self, _end: End) {}
}
