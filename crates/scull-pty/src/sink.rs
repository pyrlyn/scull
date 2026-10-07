//! The contract between the reader thread and whatever owns the terminal state.
//! Separate so this crate stays a leaf: the terminal crate never sees a PTY and
//! this crate never sees a grid.

use std::time::Instant;

use crate::limits::UNKNOWN_EXIT_CODE;

/// How the child ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitStatus {
    /// The exit code; [`crate::limits::UNKNOWN_EXIT_CODE`] when unknown, and 1
    /// on Unix when a signal ended the child.
    pub code: u32,
    /// The signal that ended the child, when one did (Unix only).
    pub signal: Option<String>,
}

impl ExitStatus {
    pub(crate) fn unknown() -> Self {
        Self {
            code: UNKNOWN_EXIT_CODE,
            signal: None,
        }
    }

    /// True when the child exited with code 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == 0 && self.signal.is_none()
    }
}

impl From<portable_pty::ExitStatus> for ExitStatus {
    fn from(status: portable_pty::ExitStatus) -> Self {
        Self {
            code: status.exit_code(),
            signal: status.signal().map(str::to_owned),
        }
    }
}

/// What the reader thread drives. All methods run on the `scull-pty-io` thread
/// with the caller's lock held, so they must not block, call into the UI or
/// take another lock.
pub trait Sink: Send + 'static {
    /// Parse and apply child output. May be called several times per lock hold,
    /// with at most [`crate::limits::MAX_LOCKED_BYTES`] bytes in all. The split
    /// between calls is arbitrary, so a sequence can be cut anywhere.
    fn feed(&mut self, bytes: &[u8]);

    /// Append the replies the child asked for (device reports, ...) to `out`, at
    /// most `limit` bytes; keep the rest queued, under the sink's own cap. Called
    /// once at the end of each lock hold.
    fn drain_replies(&mut self, out: &mut Vec<u8>, limit: usize);

    /// The child ended and all its output has been fed. Called once, last.
    fn child_exited(&mut self, status: ExitStatus);

    /// Whether the UI should be woken for what the sink holds now. Asked at
    /// the end of every hold and once [`Self::deadline`] passes; false skips
    /// the wakeup, as while synchronized output holds the picture.
    fn wants_wakeup(&mut self, now: Instant) -> bool {
        let _ = now;
        true
    }

    /// When to ask [`Self::wants_wakeup`] again although no output came: the
    /// end of a hold the sink keeps. Asked right after it.
    fn deadline(&self) -> Option<Instant> {
        None
    }
}
