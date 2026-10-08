//! Polled host events, capped because they are PTY bytes. The bell is the
//! flag `take_bell` already exposed: several BELs are one event, and
//! either door consumes it (`research.md` §7).

use std::collections::VecDeque;

/// Events waiting at once. A newer one that does not fit is dropped whole.
pub(crate) const MAX_EVENTS: usize = 32;
/// Bytes of one text payload. Longer text is refused whole.
pub(crate) const MAX_TEXT: usize = 1024;
/// Decoded OSC 52 bytes. A larger paste is dropped whole.
pub(crate) const MAX_CLIPBOARD: usize = 1 << 20;
/// Reply bytes for an OSC 52 answer: base64 of [`MAX_CLIPBOARD`] plus the
/// `ESC ] 52 ; c ;` and `ESC \` around it, so a full answer fits.
pub(crate) const MAX_CLIPBOARD_REPLY: usize = MAX_CLIPBOARD.div_ceil(3) * 4 + 16;
/// OSC 52 reads the host has not answered. More are refused at once, since
/// a host that polls but never answers would let them pile up.
pub(crate) const MAX_PENDING_CLIPS: usize = MAX_EVENTS;
/// OSC 8 targets. Rows store the id; the URI lives here.
pub(crate) const MAX_LINKS: usize = 1024;
/// URI bytes for one link.
pub(crate) const MAX_URI: usize = 2083;
/// Explicit OSC 8 `id=` bytes.
pub(crate) const MAX_LINK_KEY: usize = 256;

/// Which title OSC 0, 1 or 2 changed. The numbers are the OSC codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleWhich {
    /// Icon name and window title.
    Both = 0,
    /// Icon name.
    Icon = 1,
    /// Window title.
    Window = 2,
}

/// One OSC 52 request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clipboard {
    /// Answer [`crate::Terminal::clipboard_reply`] or `clipboard_deny` with this.
    pub id: u64,
    /// Selection parameter: `c`, `p`, `q`, `s` or `0`–`7`.
    pub selection: u8,
    /// `Pd` was `?`. A write carries `data`.
    pub read: bool,
    /// Decoded bytes of a write. Empty for a read.
    pub data: Vec<u8>,
}

/// One thing the host should notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// BEL, coalesced.
    Bell,
    /// OSC 0, 1 or 2.
    Title {
        /// Which name changed.
        which: TitleWhich,
        /// The new text.
        text: String,
    },
    /// OSC 7.
    WorkingDirectory(String),
    /// OSC 133. `mark` is a single letter; `extra` is often an exit code.
    Shell {
        /// The marker letter.
        mark: char,
        /// Bytes after the marker.
        extra: String,
    },
    /// OSC 52. The host may deny a read.
    Clipboard(Clipboard),
    /// OSC 8 opened a hyperlink.
    Link {
        /// [`scull_grid::LinkId`] number.
        id: u32,
        /// The URI.
        uri: String,
    },
}

/// The queue plus the coalesced bell flag.
#[derive(Clone, Debug, Default)]
pub(crate) struct Queue {
    items: VecDeque<Event>,
    bell: bool,
    next_clip: u64,
}

impl Queue {
    /// Queues `event`, or drops it when full.
    pub(crate) fn push(&mut self, event: Event) -> bool {
        if self.items.len() >= MAX_EVENTS {
            return false;
        }
        self.items.push_back(event);
        true
    }

    /// One BEL, however many arrived. A full queue still remembers it.
    pub(crate) fn ring(&mut self) {
        if self.bell {
            return;
        }
        self.bell = true;
        if self.items.len() < MAX_EVENTS {
            self.items.push_back(Event::Bell);
        }
    }

    pub(crate) fn poll(&mut self) -> Option<Event> {
        if let Some(event) = self.items.pop_front() {
            if matches!(event, Event::Bell) {
                self.bell = false;
            }
            return Some(event);
        }
        self.bell.then(|| {
            self.bell = false;
            Event::Bell
        })
    }

    /// Either door takes the one bell, so a later poll does not ring again.
    pub(crate) fn take_bell(&mut self) -> bool {
        let had = self.bell;
        self.bell = false;
        self.items.retain(|event| !matches!(event, Event::Bell));
        had
    }

    pub(crate) fn next_clip(&mut self) -> u64 {
        self.next_clip = self.next_clip.wrapping_add(1).max(1);
        self.next_clip
    }
}

/// UTF-8 within [`MAX_TEXT`], or nothing.
pub(crate) fn owned_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    (text.len() <= MAX_TEXT).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_drops_the_newest_and_still_rings() {
        let mut queue = Queue::default();
        for i in 0..MAX_EVENTS {
            assert!(queue.push(Event::WorkingDirectory(format!("u{i}"))));
        }
        assert!(!queue.push(Event::WorkingDirectory("nope".into())));
        queue.ring();
        assert!(queue.take_bell());
        assert!(!queue.take_bell());
        let mut n = 0;
        while queue.poll().is_some() {
            n += 1;
        }
        assert_eq!(n, MAX_EVENTS);
        queue.ring();
        queue.ring();
        assert_eq!(queue.poll(), Some(Event::Bell));
        assert_eq!(queue.poll(), None);
    }
}
