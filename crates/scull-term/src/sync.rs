//! Synchronized output (mode 2026): while a program draws between BSU
//! (`CSI ? 2026 h`) and ESU (`CSI ? 2026 l`), the frame keeps the last
//! picture so the UI never shows half a redraw. Separate from the modes
//! because the hold has caps, and one of them needs a clock that the caller
//! passes in, so tests are deterministic.
//!
//! The bytes are applied to the grid at once and only the frame update is
//! held, as foot (`render.c:5165`) and Contour (`Terminal.cpp:4661-4670`)
//! do (`docs/research/foot-contour.md` §1.11, §1.12). Alacritty instead
//! buffers the held bytes inside its parser (`vte/src/ansi.rs`, 0.15.0);
//! holding the frame needs no buffer, and the grid already caps what the
//! bytes can grow. Its caps are the ones taken here: an untrusted PTY can
//! set the mode and never reset it, so the hold ends after
//! [`MAX_SYNC_BYTES`] bytes or [`MAX_SYNC_HOLD`], and the mode then reads
//! as reset, as Alacritty reports it.

use std::time::{Duration, Instant};

/// Bytes a synchronized update may span before the picture is released:
/// Alacritty's `SYNC_BUFFER_SIZE` (2 MiB), several full redraws of a large
/// screen.
pub const MAX_SYNC_BYTES: usize = 2 * 1024 * 1024;

/// How long a synchronized update may hold the picture: Alacritty's
/// `SYNC_UPDATE_TIMEOUT` and Contour's default (150 ms). foot waits 1 s; a
/// shorter hold keeps a program that forgets ESU from freezing the view.
pub const MAX_SYNC_HOLD: Duration = Duration::from_millis(150);

/// What the current hold has used of its caps.
///
/// The bytes are counted per `feed` call that ends with the mode set, so a
/// call that both starts and ends inside one update counts whole. The time
/// starts at the first look that finds the mode set (the reader thread looks
/// right after each `feed`), so a hold is timed from the first chunk that
/// set it. An ESU and a new BSU inside one chunk are never seen apart; the
/// caps then run on, which is what guarantees the picture moves at all.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SyncGate {
    since: Option<Instant>,
    bytes: usize,
}

impl SyncGate {
    /// Counts `len` bytes just fed; `active` is the mode after them. Returns
    /// whether the byte cap is passed and the hold must end.
    pub(crate) fn fed(&mut self, active: bool, len: usize) -> bool {
        if !active {
            *self = Self::default();
            return false;
        }
        self.bytes = self.bytes.saturating_add(len);
        self.bytes > MAX_SYNC_BYTES
    }

    /// Whether the picture is still held at `now`. False once the mode is
    /// reset or the hold has lasted [`MAX_SYNC_HOLD`].
    pub(crate) fn holds(&mut self, active: bool, now: Instant) -> bool {
        if !active {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.saturating_duration_since(since) < MAX_SYNC_HOLD
    }

    /// When the hold ends by itself, once it has been seen.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.since.and_then(|s| s.checked_add(MAX_SYNC_HOLD))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inactive_mode_never_holds_and_forgets_the_hold() {
        let now = Instant::now();
        let mut g = SyncGate::default();
        assert!(g.holds(true, now));
        assert!(!g.fed(true, 1));
        assert!(!g.holds(false, now));
        assert_eq!(g.deadline(), None);
        assert_eq!(g.bytes, 0);
    }

    #[test]
    fn the_hold_is_timed_from_the_first_look() {
        let start = Instant::now();
        let mut g = SyncGate::default();
        assert!(!g.fed(true, 10));
        assert_eq!(g.deadline(), None, "not seen yet");
        assert!(g.holds(true, start));
        assert_eq!(g.deadline(), start.checked_add(MAX_SYNC_HOLD));
        let almost = MAX_SYNC_HOLD - Duration::from_millis(1);
        assert!(g.holds(true, start + almost));
        assert!(!g.holds(true, start + MAX_SYNC_HOLD));
    }

    #[test]
    fn bytes_add_up_across_feeds_until_the_cap() {
        let mut g = SyncGate::default();
        assert!(!g.fed(true, MAX_SYNC_BYTES));
        assert!(g.fed(true, 1));
        assert!(!g.fed(false, 1), "a reset mode starts over");
        assert!(!g.fed(true, MAX_SYNC_BYTES));
    }
}
