//! OSC that is not an image: titles, directory, shell marks, clipboard,
//! hyperlinks and notifications, from the published specs (xterm ctlseqs,
//! OSC 8, OSC 7, OSC 133, iTerm2's OSC 9, rxvt-unicode's OSC 777 `notify`).
//! Nothing here is taken from kitty.

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use scull_grid::LinkId;
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
    id: LinkId,
    key: String,
    uri: String,
}

/// The open link and the URIs ids point at.
#[derive(Clone, Debug, Default)]
pub(crate) struct Links {
    items: Vec<Hyperlink>,
    pub(crate) current: Option<LinkId>,
}

impl Links {
    pub(crate) fn close(&mut self) {
        self.current = None;
    }

    fn uri(&self, id: u32) -> Option<&str> {
        self.items
            .iter()
            .find(|link| link.id.0 == id)
            .map(|link| link.uri.as_str())
    }

    /// An explicit id with the same URI is one link, so adjacent runs join.
    /// No id means a new link: the id is what says two runs are the same.
    fn open(&mut self, key: &str, uri: &str) -> Option<LinkId> {
        if !key.is_empty()
            && let Some(id) = self
                .items
                .iter()
                .find(|link| link.key == key && link.uri == uri)
                .map(|link| link.id)
        {
            return (self.current.replace(id) != Some(id)).then_some(id);
        }
        // A link with no slot leaves its text unlinked; keeping the open
        // link would stamp the old URI onto it.
        self.current = None;
        if self.items.len() >= MAX_LINKS {
            return None;
        }
        let Ok(n) = u32::try_from(self.items.len()) else {
            return None;
        };
        let id = LinkId(n.saturating_add(1));
        if id.0 == 0 {
            return None;
        }
        self.items.push(Hyperlink {
            id,
            key: key.to_owned(),
            uri: uri.to_owned(),
        });
        self.current = Some(id);
        Some(id)
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
        if let Some(id) = self.links.open(&key, &uri) {
            let _ = self.events.push(Event::Link { id: id.0, uri });
        }
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

    pub(crate) fn clipboard_reply(&mut self, id: u64, data: &[u8]) -> bool {
        if data.len() > MAX_CLIPBOARD {
            return false;
        }
        let Some(pending) = self.take_clip(id) else {
            return false;
        };
        self.reply_clip(pending.selection, data, pending.bell)
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

    /// Answers an OSC 52 read. False if `id` is not open or `data` is over the
    /// cap; also false, with an empty answer sent, when the reply queue is full.
    pub fn clipboard_reply(&mut self, id: u64, data: &[u8]) -> bool {
        self.state.clipboard_reply(id, data)
    }
}

#[cfg(test)]
mod tests {
    use crate::events::Event;
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
        assert!(term.clipboard_reply(clip.id, b"hi"));
        assert_eq!(term.take_replies(), b"\x1b]52;p;aGk=\x1b\\");
    }

    #[test]
    fn a_link_with_no_slot_leaves_its_text_unlinked() {
        let mut term = term();
        for i in 0..crate::events::MAX_LINKS {
            term.feed(format!("\x1b]8;;u{i}\x1b\\").as_bytes());
        }
        term.feed(b"\x1b]8;;full\x1b\\x\x1b]8;;\x1b\\");
        let row = term.grid().screen_row(0).unwrap();
        assert_eq!(row.link_at(0), None);
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
        let data = vec![b'a'; crate::events::MAX_CLIPBOARD];
        assert!(term.clipboard_reply(clip.id, &data));
        let reply = term.take_replies();
        assert!(reply.starts_with(b"\x1b]52;c;YWFh"));
        assert!(reply.ends_with(b"\x07"));
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
}
