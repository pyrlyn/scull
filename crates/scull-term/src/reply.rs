//! Replies to the program: device attributes, status and cursor reports,
//! mode reports. Separate because it is the only output a PTY can make the
//! core produce, so the queue that holds it is where the cap lives.
//!
//! Formats follow xterm's ctlseqs (DA1, DA2, DSR, DECXCPR, DECRQM). The
//! queue holds whole replies only: when one would pass the cap it is
//! dropped, never cut, so the program never reads half a sequence.

use crate::modes::{AnsiMode, DecMode, ModeStatus};
use crate::state::State;

/// The most reply bytes held until the caller drains them. Far above what
/// any program waits for between reads, small enough that a flood of
/// requests from an untrusted PTY cannot grow memory.
pub(crate) const MAX_REPLY_BYTES: usize = 4096;

/// DA1: a VT220 (62) with ANSI colour (22), the set xterm's VT220 mode
/// reports minus the features this core lacks.
const PRIMARY_ATTRIBUTES: &[u8] = b"\x1b[?62;22c";

/// DA2: terminal type 1 (VT220), firmware version 0 until Scull has a
/// release number, ROM cartridge 0.
const SECONDARY_ATTRIBUTES: &[u8] = b"\x1b[>1;0;0c";

/// DSR 5: "OK, no malfunction".
const STATUS_OK: &[u8] = b"\x1b[0n";

/// DECXCPR reports the page; there is only one.
const PAGE: u16 = 1;

/// DSR parameters.
pub(crate) const DSR_STATUS: u16 = 5;
pub(crate) const DSR_CURSOR: u16 = 6;

/// Reply bytes waiting for the caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct Replies {
    bytes: Vec<u8>,
}

impl Replies {
    /// Queues one whole reply, or drops it if the queue is full.
    pub(crate) fn push(&mut self, reply: &[u8]) {
        if self.bytes.len() + reply.len() <= MAX_REPLY_BYTES {
            self.bytes.extend_from_slice(reply);
        }
    }

    pub(crate) fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

impl State {
    pub(crate) fn primary_attributes(&mut self) {
        self.replies.push(PRIMARY_ATTRIBUTES);
    }

    pub(crate) fn secondary_attributes(&mut self) {
        self.replies.push(SECONDARY_ATTRIBUTES);
    }

    /// DSR (`CSI Ps n`) and, with `private`, DECDSR (`CSI ? Ps n`).
    pub(crate) fn device_status(&mut self, ps: u16, private: bool) {
        match (ps, private) {
            (DSR_STATUS, false) => self.replies.push(STATUS_OK),
            (DSR_CURSOR, _) => {
                let (row, col) = self.reported_position();
                let reply = if private {
                    format!("\x1b[?{row};{col};{PAGE}R")
                } else {
                    format!("\x1b[{row};{col}R")
                };
                self.replies.push(reply.as_bytes());
            }
            _ => {}
        }
    }

    /// The 1-based cursor position a report gives: relative to the margins
    /// in origin mode, as xterm reports it.
    fn reported_position(&self) -> (u16, u16) {
        let (mut row, mut col) = (self.cursor.row, self.cursor.col);
        if self.modes.origin {
            row = row.saturating_sub(self.margins.top);
            col = col.saturating_sub(self.margins.left);
        }
        (row.saturating_add(1), col.saturating_add(1))
    }

    /// DECRQM for an ANSI mode: `CSI Pd ; Ps $ y`.
    pub(crate) fn report_ansi_mode(&mut self, code: u16) {
        let status = match AnsiMode::from_code(code) {
            Some(AnsiMode::Insert) => self.modes.insert.into(),
            Some(AnsiMode::Newline) => self.modes.newline.into(),
            None => ModeStatus::NotRecognized,
        };
        let reply = format!("\x1b[{code};{}$y", status as u16);
        self.replies.push(reply.as_bytes());
    }

    /// DECRQM for a DEC private mode: `CSI ? Pd ; Ps $ y`.
    pub(crate) fn report_dec_mode(&mut self, code: u16) {
        let status = DecMode::from_code(code).map_or(ModeStatus::NotRecognized, |mode| {
            ModeStatus::from(self.dec_mode(mode))
        });
        let reply = format!("\x1b[?{code};{}$y", status as u16);
        self.replies.push(reply.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_never_passes_its_cap_and_keeps_replies_whole() {
        let mut r = Replies::default();
        for _ in 0..MAX_REPLY_BYTES {
            r.push(STATUS_OK);
        }
        let bytes = r.take();
        assert!(bytes.len() <= MAX_REPLY_BYTES);
        assert_eq!(bytes.len() % STATUS_OK.len(), 0);
        assert!(bytes.chunks(STATUS_OK.len()).all(|c| c == STATUS_OK));
    }

    #[test]
    fn taking_the_replies_empties_the_queue() {
        let mut r = Replies::default();
        r.push(STATUS_OK);
        assert_eq!(r.take(), STATUS_OK);
        assert!(r.take().is_empty());
    }

    #[test]
    fn a_reply_too_big_for_the_room_left_is_dropped_whole() {
        let mut r = Replies::default();
        r.push(&[b'x'; MAX_REPLY_BYTES - 2]);
        r.push(STATUS_OK);
        assert_eq!(r.take().len(), MAX_REPLY_BYTES - 2);
    }
}
