//! OSC that is not an image: titles, directory, shell marks, clipboard,
//! hyperlinks and notifications, from the published specs (xterm ctlseqs,
//! OSC 8, OSC 7, OSC 133, iTerm2's OSC 9, rxvt-unicode's OSC 777 `notify`).
//! Nothing here is taken from kitty.

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use scull_grid::{LinkId, Marks};
use scull_parser::Osc;

use crate::events::{
    Clipboard, Event, MAX_CLIPBOARD, MAX_LINK_KEY, MAX_LINKS, MAX_PENDING_CLIPS, MAX_URI,
    TitleWhich, owned_text,
};
use crate::state::State;

/// Clients omit the final `=`; image payloads already accept that.
const B64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// A link sweep walks every row of both screens, so it runs only after
/// `MAX_LINKS / LINK_SWEEP_DIVISOR` opens since the last one, the grid's
/// rule for its own tables: the walk stays a constant factor of the work.
const LINK_SWEEP_DIVISOR: usize = 8;

/// ConEmu's OSC 9 subcommands are numbered up to this.
const CONEMU_LAST: u8 = 12;

/// A read the host has not answered.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    id: u64,
    selection: u8,
    bell: bool,
}

#[derive(Clone, Debug)]
struct Hyperlink {
    key: String,
    uri: String,
}

/// Every id is taken; a sweep may free some.
struct NoSlot;

/// The open link and the URIs ids point at. Slot `n` is id `n + 1`, so 0
/// stays "no link"; an empty slot is an id a sweep freed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Links {
    slots: Vec<Option<Hyperlink>>,
    free: Vec<u32>,
    pub(crate) current: Option<LinkId>,
    /// New ids asked for since the last sweep, granted or not.
    opens_since_sweep: usize,
}

fn slot_of(id: u32) -> Option<usize> {
    usize::try_from(id.checked_sub(1)?).ok()
}

impl Links {
    pub(crate) fn close(&mut self) {
        self.current = None;
    }

    fn uri(&self, id: u32) -> Option<&str> {
        let link = self.slots.get(slot_of(id)?)?.as_ref()?;
        Some(link.uri.as_str())
    }

    /// An explicit id with the same URI is one link, so adjacent runs join.
    /// No id means a new link: the id is what says two runs are the same.
    /// `Ok(None)`: that link is already the open one.
    fn open(&mut self, key: &str, uri: &str) -> Result<Option<LinkId>, NoSlot> {
        if !key.is_empty()
            && let Some(id) = self.find(key, uri)
        {
            return Ok((self.current.replace(id) != Some(id)).then_some(id));
        }
        // A link with no slot leaves its text unlinked; keeping the open
        // link would stamp the old URI onto it.
        self.current = None;
        self.opens_since_sweep += 1;
        let link = Hyperlink {
            key: key.to_owned(),
            uri: uri.to_owned(),
        };
        let id = if let Some(id) = self.free.pop() {
            let slot = slot_of(id)
                .and_then(|at| self.slots.get_mut(at))
                .ok_or(NoSlot)?;
            *slot = Some(link);
            id
        } else if self.slots.len() < MAX_LINKS {
            self.slots.push(Some(link));
            u32::try_from(self.slots.len()).map_err(|_| NoSlot)?
        } else {
            return Err(NoSlot);
        };
        self.current = Some(LinkId(id));
        Ok(Some(LinkId(id)))
    }

    fn find(&self, key: &str, uri: &str) -> Option<LinkId> {
        self.slots.iter().zip(1..).find_map(|(slot, id)| {
            slot.as_ref()
                .filter(|link| link.key == key && link.uri == uri)
                .map(|_| LinkId(id))
        })
    }

    fn sweep_pays(&self) -> bool {
        self.opens_since_sweep * LINK_SWEEP_DIVISOR >= MAX_LINKS
    }

    /// Marks with the open link kept: its text may not be printed yet.
    fn marks(&self) -> Marks {
        let mut live = Marks::new(self.slots.len() + 1);
        if let Some(id) = self.current {
            live.mark(id.0);
        }
        live
    }

    /// Frees every id `live` does not mark. Nothing between marking and
    /// sweeping can open a link, so the grid's insert stamp is not needed.
    fn sweep(&mut self, live: &Marks) {
        for (slot, id) in self.slots.iter_mut().zip(1u32..) {
            if slot.is_some() && !live.is_marked(id as usize) {
                *slot = None;
                self.free.push(id);
            }
        }
        self.opens_since_sweep = 0;
    }
}

impl State {
    pub(crate) fn osc(&mut self, osc: &Osc<'_>) {
        if osc.data.starts_with(b"1337;") {
            self.osc_image(osc.data);
            return;
        }
        let Some((code, rest)) = split_semi(osc.data) else {
            return;
        };
        match code {
            b"0" | b"1" | b"2" => self.osc_title(code, rest),
            b"7" => self.osc_directory(rest),
            b"8" => self.osc_link(rest),
            b"9" if !conemu(rest) => self.osc_notify(b"", rest),
            b"52" => self.osc_clipboard(rest, osc.bell_terminated),
            b"133" => self.osc_shell(rest),
            b"777" => {
                if let Some((b"notify", args)) = split_semi(rest) {
                    let (title, body) = split_semi(args).unwrap_or((args, b""));
                    self.osc_notify(title, body);
                }
            }
            _ => {}
        }
    }

    fn osc_title(&mut self, code: &[u8], rest: &[u8]) {
        let Some(text) = owned_text(rest) else {
            return;
        };
        let which = match code {
            b"0" => TitleWhich::Both,
            b"1" => TitleWhich::Icon,
            b"2" => TitleWhich::Window,
            _ => return,
        };
        let _ = self.events.push(Event::Title { which, text });
    }

    fn osc_directory(&mut self, rest: &[u8]) {
        let Some(uri) = owned_text(rest) else {
            return;
        };
        let _ = self.events.push(Event::WorkingDirectory(uri));
    }

    fn osc_shell(&mut self, rest: &[u8]) {
        let (mark, extra) = split_semi(rest).unwrap_or((rest, b""));
        let [mark] = *mark else {
            return;
        };
        if !mark.is_ascii_alphabetic() {
            return;
        }
        let Some(extra) = owned_text(extra) else {
            return;
        };
        let _ = self.events.push(Event::Shell {
            mark: char::from(mark),
            extra,
        });
    }

    fn osc_link(&mut self, rest: &[u8]) {
        let (params, uri) = split_semi(rest).unwrap_or((rest, b""));
        if uri.is_empty() {
            self.links.close();
            return;
        }
        let (Some(key), Some(uri)) = (link_key(params), uri_text(uri)) else {
            return;
        };
        let opened = match self.links.open(&key, &uri) {
            Err(NoSlot) if self.links.sweep_pays() => {
                self.sweep_links();
                self.links.open(&key, &uri)
            }
            other => other,
        };
        if let Ok(Some(id)) = opened {
            let _ = self.events.push(Event::Link { id: id.0, uri });
        }
    }

    /// Frees the link ids no row of either screen holds, history included,
    /// so an id is never reused while text still points at it.
    fn sweep_links(&mut self) {
        let mut live = self.links.marks();
        for grid in [&self.grid, &self.alt] {
            let rows = grid.history_len() + usize::from(grid.screen_rows());
            for row in (0..rows).filter_map(|i| grid.row(i)) {
                for span in row.links() {
                    live.mark(span.link.0);
                }
            }
        }
        self.links.sweep(&live);
    }

    fn osc_notify(&mut self, title: &[u8], body: &[u8]) {
        let (Some(title), Some(body)) = (owned_text(title), owned_text(body)) else {
            return;
        };
        if title.is_empty() && body.is_empty() {
            return;
        }
        let _ = self.events.push(Event::Notification { title, body });
    }

    fn osc_clipboard(&mut self, rest: &[u8], bell: bool) {
        let (pc, pd) = split_semi(rest).unwrap_or((rest, b""));
        let selection = selection_byte(pc);
        if pd == b"?" {
            let id = self.events.next_clip();
            let queued = self.clips.len() < MAX_PENDING_CLIPS
                && self.events.push(Event::Clipboard(Clipboard {
                    id,
                    selection,
                    read: true,
                    data: Vec::new(),
                }));
            if queued {
                self.clips.push(Pending {
                    id,
                    selection,
                    bell,
                });
            } else {
                // The host will never see the read, so answer empty.
                self.reply_clip(selection, &[], bell);
            }
            return;
        }
        let Some(data) = decode_clip(pd) else {
            return;
        };
        let id = self.events.next_clip();
        let _ = self.events.push(Event::Clipboard(Clipboard {
            id,
            selection,
            read: false,
            data,
        }));
    }

    pub(crate) fn clipboard_deny(&mut self, id: u64) -> bool {
        let Some(pending) = self.take_clip(id) else {
            return false;
        };
        self.reply_clip(pending.selection, &[], pending.bell);
        true
    }

    pub(crate) fn clipboard_reply(&mut self, id: u64, data: &[u8]) -> Option<bool> {
        if data.len() > MAX_CLIPBOARD {
            return None;
        }
        let pending = self.take_clip(id)?;
        Some(self.reply_clip(pending.selection, data, pending.bell))
    }

    fn take_clip(&mut self, id: u64) -> Option<Pending> {
        let at = self.clips.iter().position(|clip| clip.id == id)?;
        Some(self.clips.remove(at))
    }

    /// False when `data` did not fit and the program got an empty answer
    /// instead, so it never waits for a reply that will not come.
    fn reply_clip(&mut self, selection: u8, data: &[u8], bell: bool) -> bool {
        if !data.is_empty() && self.replies.push_host(&clip_reply(selection, data, bell)) {
            return true;
        }
        self.replies.push(&clip_reply(selection, &[], bell));
        data.is_empty()
    }
}

fn clip_reply(selection: u8, data: &[u8], bell: bool) -> Vec<u8> {
    let encoded = B64.encode(data);
    let mut reply = Vec::with_capacity(8 + encoded.len());
    reply.extend_from_slice(b"\x1b]52;");
    reply.push(selection);
    reply.push(b';');
    reply.extend(encoded.into_bytes());
    if bell {
        reply.push(0x07);
    } else {
        reply.extend_from_slice(b"\x1b\\");
    }
    reply
}

/// ConEmu's OSC 9 subcommands (`9;4;state;progress` and the rest) start
/// with their number. They are not notification text, and this core does
/// not implement them, so they are dropped rather than shown.
fn conemu(rest: &[u8]) -> bool {
    let number = split_semi(rest).map_or(rest, |(first, _)| first);
    std::str::from_utf8(number)
        .ok()
        .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=CONEMU_LAST).contains(&n))
}

fn split_semi(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|byte| *byte == b';')?;
    Some((&bytes[..at], &bytes[at + 1..]))
}

fn link_key(params: &[u8]) -> Option<String> {
    let mut key = String::new();
    for part in params.split(|&b| b == b':') {
        let Some(raw) = part.strip_prefix(b"id=") else {
            continue;
        };
        if raw.len() > MAX_LINK_KEY {
            return None;
        }
        key = std::str::from_utf8(raw).ok()?.to_owned();
    }
    Some(key)
}

fn uri_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    (text.len() <= MAX_URI).then(|| text.to_owned())
}

fn selection_byte(pc: &[u8]) -> u8 {
    pc.iter()
        .copied()
        .find(|byte| matches!(byte, b'c' | b'p' | b'q' | b's' | b'0'..=b'7'))
        .unwrap_or(b'c')
}

fn decode_clip(pd: &[u8]) -> Option<Vec<u8>> {
    let max_in = MAX_CLIPBOARD / 3 * 4 + 4;
    if pd.len() > max_in {
        return None;
    }
    let mut out = Vec::new();
    B64.decode_vec(pd, &mut out).ok()?;
    (out.len() <= MAX_CLIPBOARD).then_some(out)
}

impl crate::terminal::Terminal {
    /// The next event, or `None` when the queue is empty.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.state.events.poll()
    }

    /// URI for a link id still known to the terminal.
    pub fn link_uri(&self, id: u32) -> Option<&str> {
        self.state.links.uri(id)
    }

    /// Refuses an OSC 52 read with an empty reply. False if `id` is not open.
    pub fn clipboard_deny(&mut self, id: u64) -> bool {
        self.state.clipboard_deny(id)
    }

    /// Answers an OSC 52 read: `Some(true)` once queued. `None`, doing
    /// nothing, if `id` is not open or `data` is over the cap; `Some(false)`,
    /// with an empty answer sent instead, when the reply queue is full.
    pub fn clipboard_reply(&mut self, id: u64, data: &[u8]) -> Option<bool> {
        self.state.clipboard_reply(id, data)
    }
}

#[cfg(test)]
mod tests {
    use scull_grid::{Content, Row};

    use crate::events::{Event, MAX_LINKS};
    use crate::terminal::Terminal;

    fn term() -> Terminal {
        Terminal::new(8, 2, 0).unwrap()
    }

    #[test]
    fn the_host_may_deny_or_answer_a_clipboard_read() {
        let mut term = term();
        term.feed(b"\x1b]52;c;?\x07");
        let Event::Clipboard(clip) = term.poll_event().unwrap() else {
            panic!("read");
        };
        assert!(clip.read);
        assert!(term.clipboard_deny(clip.id));
        assert!(!term.clipboard_deny(clip.id));
        assert_eq!(term.take_replies(), b"\x1b]52;c;\x07");
        term.feed(b"\x1b]52;p;?\x1b\\");
        let Event::Clipboard(clip) = term.poll_event().unwrap() else {
            panic!("read");
        };
        assert_eq!(term.clipboard_reply(clip.id, b"hi"), Some(true));
        assert_eq!(term.clipboard_reply(clip.id, b"hi"), None, "answered once");
        assert_eq!(term.take_replies(), b"\x1b]52;p;aGk=\x1b\\");
    }

    #[test]
    fn a_link_with_no_slot_leaves_its_text_unlinked() {
        let cols = 8;
        let rows = MAX_LINKS / cols;
        let mut term = Terminal::new(cols as u16, 2, rows).unwrap();
        // Every id is held by a printed cell, so a sweep frees nothing.
        for i in 0..MAX_LINKS {
            term.feed(format!("\x1b]8;;u{i}\x1b\\x").as_bytes());
        }
        term.feed(b"\x1b]8;;\x1b\\\r\n");
        term.feed(b"\x1b]8;;full\x1b\\y\x1b]8;;\x1b\\");
        let row = term.grid().screen_row(1).unwrap();
        assert_eq!(row.link_at(0), None);
        assert_eq!(term.link_uri(1), Some("u0"));
    }

    fn row_text(row: &Row) -> String {
        row.cells()
            .filter_map(|cell| match cell.content() {
                Content::Char(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn link_ids_no_row_holds_are_reused_and_live_ones_stay_right() {
        let mut term = Terminal::new(8, 3, 20).unwrap();
        // Some links live on the alternate screen and must survive there.
        term.feed(b"\x1b[?47h\x1b]8;;alt\x1b\\A\x1b]8;;\x1b\\\x1b[?47l");
        let mut opened = 0;
        for i in 0..5 * MAX_LINKS {
            term.feed(format!("\x1b]8;;u{i}\x1b\\{i}\x1b]8;;\x1b\\\r\n").as_bytes());
            let printed = term.grid().screen_row(1).unwrap();
            if let Some(id) = printed.link_at(0) {
                opened += 1;
                assert_eq!(term.link_uri(id.0), Some(format!("u{i}").as_str()));
            }
        }
        // Each sweep waits an eighth of the table's opens, and the ids
        // still on screen and in history are never freed.
        assert!(opened > 4 * MAX_LINKS, "only {opened} linked");
        let grid = term.grid();
        for row in (0..grid.history_len() + 3).filter_map(|i| grid.row(i)) {
            if let Some(id) = row.link_at(0) {
                let uri = term.link_uri(id.0).unwrap();
                assert_eq!(uri, format!("u{}", row_text(row)));
            }
        }
        term.feed(b"\x1b[?47h");
        let id = term.grid().screen_row(0).unwrap().link_at(0).unwrap();
        assert_eq!(term.link_uri(id.0), Some("alt"));
    }

    #[test]
    fn conemu_subcommands_are_not_notifications() {
        let mut term = term();
        term.feed(b"\x1b]9;4;1;50\x07\x1b]9;9;/tmp\x07\x1b]9;\x07\x1b]777;precmd\x07");
        assert_eq!(term.poll_event(), None);
        term.feed(b"\x1b]9;13 done\x07");
        assert_eq!(
            term.poll_event(),
            Some(Event::Notification {
                title: String::new(),
                body: "13 done".into()
            })
        );
    }

    #[test]
    fn unanswered_clipboard_reads_are_capped_and_answered_empty() {
        let mut term = term();
        for _ in 0..crate::events::MAX_PENDING_CLIPS {
            term.feed(b"\x1b]52;c;?\x07");
            while term.poll_event().is_some() {}
        }
        assert!(term.take_replies().is_empty());
        term.feed(b"\x1b]52;c;?\x07");
        assert_eq!(term.poll_event(), None);
        assert_eq!(term.take_replies(), b"\x1b]52;c;\x07");
    }

    #[test]
    fn a_full_size_clipboard_answer_reaches_the_program() {
        let mut term = term();
        term.feed(b"\x1b]52;c;?\x07");
        let Some(Event::Clipboard(clip)) = term.poll_event() else {
            panic!("read");
        };
        let mut data = vec![b'a'; crate::events::MAX_CLIPBOARD + 1];
        assert_eq!(term.clipboard_reply(clip.id, &data), None, "over the cap");
        data.pop();
        assert_eq!(term.clipboard_reply(clip.id, &data), Some(true));
        let reply = term.take_replies();
        assert!(reply.starts_with(b"\x1b]52;c;YWFh"));
        assert!(reply.ends_with(b"\x07"));
    }
}
