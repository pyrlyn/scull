//! The polled event queue: what happened besides the picture changing.
//! Polled rather than called back so the host handles events on its own
//! thread, with no core lock held and no reentrancy into the core.
//!
//! An event's text (a title, a URI, clipboard bytes) is not in `tt_event`:
//! the handle keeps the text of the event polled last, and the host copies
//! it out with `tt_term_event_text`, so the host frees nothing and the event
//! struct stays fixed-size.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::slice;

use scull_pty::limits::UNKNOWN_EXIT_CODE;
use scull_term::{Event, TitleWhich};

use crate::guard::{SizedStruct, can_write, tt_status, with_term, write_sized, zeroed};
use crate::term::{Core, tt_term};
use crate::text::{copy_out, out_ok};

/// `tt_event.kind`: the program rang the bell, once or more since the last
/// poll.
pub const TT_EVENT_BELL: u32 = 1;

/// `tt_event.kind`: the child ended and all its output has been parsed.
/// Reported once, last.
pub const TT_EVENT_CHILD_EXITED: u32 = 2;

/// `tt_event.kind`: OSC 0, 1 or 2 named the window or icon. `detail` is a
/// `TT_TITLE_*`; the text is the name.
pub const TT_EVENT_TITLE: u32 = 3;

/// `tt_event.kind`: OSC 7 reported the working directory; the text is its
/// `file://` URI as the program sent it.
pub const TT_EVENT_WORKING_DIRECTORY: u32 = 4;

/// `tt_event.kind`: an OSC 133 shell mark. `detail` is the marker letter
/// (`A` prompt, `B` command, `C` output, `D` done); the text is what
/// followed it, often an exit code.
pub const TT_EVENT_SHELL_MARK: u32 = 5;

/// `tt_event.kind`: OSC 52 asks for the clipboard. `id` is the request:
/// answer it with `tt_term_clipboard_reply` or `tt_term_clipboard_deny`.
/// `detail` is the selection byte (`c`, `p`, `q`, `s`, `0`-`7`).
pub const TT_EVENT_CLIPBOARD_READ: u32 = 6;

/// `tt_event.kind`: OSC 52 sets the clipboard; the text is the decoded
/// bytes, which need not be UTF-8. `detail` is the selection byte.
pub const TT_EVENT_CLIPBOARD_WRITE: u32 = 7;

/// `tt_event.kind`: OSC 8 opened a hyperlink. `id` is its link id; the text
/// is the URI. `tt_term_link_uri` resolves the id later.
pub const TT_EVENT_LINK: u32 = 8;

/// `tt_event.kind`: OSC 9 or OSC 777 `notify` asks for a notification. The
/// text is the message, `TT_EVENT_TEXT_TITLE` its title (empty for OSC 9).
pub const TT_EVENT_NOTIFICATION: u32 = 9;

/// `tt_event.detail` of `TT_EVENT_TITLE`: OSC 0, icon name and window title.
pub const TT_TITLE_BOTH: u32 = 0;
/// `tt_event.detail` of `TT_EVENT_TITLE`: OSC 1, the icon name.
pub const TT_TITLE_ICON: u32 = 1;
/// `tt_event.detail` of `TT_EVENT_TITLE`: OSC 2, the window title.
pub const TT_TITLE_WINDOW: u32 = 2;

/// `tt_term_event_text` part: the event's text, `text_len` bytes.
pub const TT_EVENT_TEXT_BODY: u32 = 0;
/// `tt_term_event_text` part: a notification's title, `title_len` bytes.
pub const TT_EVENT_TEXT_TITLE: u32 = 1;

/// `tt_event.exit_code` when the system did not say. A literal, not the
/// PTY crate's constant, so the generated header can spell it.
pub const TT_EXIT_CODE_UNKNOWN: u32 = 0xFFFF_FFFF;

const _: () = assert!(TT_EXIT_CODE_UNKNOWN == UNKNOWN_EXIT_CODE);
const _: () = assert!(TitleWhich::Both as u32 == TT_TITLE_BOTH);
const _: () = assert!(TitleWhich::Icon as u32 == TT_TITLE_ICON);
const _: () = assert!(TitleWhich::Window as u32 == TT_TITLE_WINDOW);

/// One event. A `u32` kind rather than an enum, so a kind added later is a
/// value an older host skips, not undefined behaviour.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_event {
    /// `sizeof(tt_event)` as the host knows it.
    pub struct_size: u32,
    /// One of `TT_EVENT_*`; skip kinds you do not know.
    pub kind: u32,
    /// `TT_EVENT_CHILD_EXITED`: the exit code, or `TT_EXIT_CODE_UNKNOWN`.
    pub exit_code: u32,
    /// `TT_EVENT_CHILD_EXITED`: 1 when a signal ended the child.
    pub signaled: u8,
    /// Names this event to `tt_term_event_text`; never 0.
    pub serial: u64,
    /// The clipboard request of `TT_EVENT_CLIPBOARD_*`, the link id of
    /// `TT_EVENT_LINK`; 0 otherwise.
    pub id: u64,
    /// Per kind: a `TT_TITLE_*`, a shell marker letter, a clipboard
    /// selection byte; 0 otherwise.
    pub detail: u32,
    /// Bytes of the text (`TT_EVENT_TEXT_BODY`); 0 when there is none.
    pub text_len: u32,
    /// Bytes of a notification's title (`TT_EVENT_TEXT_TITLE`).
    pub title_len: u32,
}

// SAFETY: repr(C), `struct_size` first, integers only.
unsafe impl SizedStruct for tt_event {}

/// The text of the event handed out last, until the next poll replaces it.
/// One event's worth: the queue's caps bound it (a clipboard write at most).
#[derive(Debug, Default)]
pub(crate) struct Polled {
    serial: u64,
    body: Vec<u8>,
    title: Vec<u8>,
}

fn text_len(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len()).unwrap_or(u32::MAX)
}

/// The fixed part of `event` and its body and title text.
fn describe(event: Event) -> (tt_event, Vec<u8>, Vec<u8>) {
    let mut out: tt_event = zeroed();
    let (kind, body, title) = match event {
        Event::Bell => (TT_EVENT_BELL, Vec::new(), Vec::new()),
        Event::Title { which, text } => {
            out.detail = which as u32;
            (TT_EVENT_TITLE, text.into_bytes(), Vec::new())
        }
        Event::WorkingDirectory(uri) => (TT_EVENT_WORKING_DIRECTORY, uri.into_bytes(), Vec::new()),
        Event::Shell { mark, extra } => {
            out.detail = u32::from(mark);
            (TT_EVENT_SHELL_MARK, extra.into_bytes(), Vec::new())
        }
        Event::Clipboard(clip) => {
            out.id = clip.id;
            out.detail = u32::from(clip.selection);
            let kind = if clip.read {
                TT_EVENT_CLIPBOARD_READ
            } else {
                TT_EVENT_CLIPBOARD_WRITE
            };
            (kind, clip.data, Vec::new())
        }
        Event::Link { id, uri } => {
            out.id = u64::from(id);
            (TT_EVENT_LINK, uri.into_bytes(), Vec::new())
        }
        Event::Notification { title, body } => {
            (TT_EVENT_NOTIFICATION, body.into_bytes(), title.into_bytes())
        }
    };
    out.kind = kind;
    out.text_len = text_len(&body);
    out.title_len = text_len(&title);
    (out, body, title)
}

impl Core {
    /// The next event with its text kept for `tt_term_event_text`, the
    /// child's exit once the queue is empty, or `None`.
    fn next_event(&mut self) -> Option<tt_event> {
        let (mut next, body, title) = match self.term.poll_event() {
            Some(event) => describe(event),
            None => {
                let exit = self.exit.take()?;
                let mut next: tt_event = zeroed();
                next.kind = TT_EVENT_CHILD_EXITED;
                next.exit_code = exit.code;
                next.signaled = u8::from(exit.signal.is_some());
                (next, Vec::new(), Vec::new())
            }
        };
        let serial = self.polled.serial.wrapping_add(1).max(1);
        self.polled = Polled {
            serial,
            body,
            title,
        };
        next.serial = serial;
        Some(next)
    }
}

/// Takes the next event into `*event`: `TT_OK` with one, `TT_EMPTY` when
/// there is none. Call it until `TT_EMPTY` after every wakeup. The event's
/// text stays readable through `tt_term_event_text` until the next poll.
///
/// # Safety
///
/// `term` is `NULL` or live; `event` is `NULL` or points to `struct_size`
/// writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_poll_event(
    term: *const tt_term,
    event: *mut tt_event,
) -> tt_status {
    let body = |t: &tt_term| {
        // Checked first: an event taken must reach the host.
        // SAFETY: the caller's contract.
        if !unsafe { can_write(event) } {
            return tt_status::TT_INVALID;
        }
        t.rearm_wakeup();
        let Some(next) = t.core().lock().next_event() else {
            return tt_status::TT_EMPTY;
        };
        // SAFETY: checked writable above.
        unsafe { write_sized(event, &next) };
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Copies part `part` (`TT_EVENT_TEXT_*`) of the text of event `serial`
/// into `buf`, as `tt_term_read_text` does: `*len` becomes its length, and
/// `TT_FULL` with nothing copied when that is more than `cap`. Only the
/// event polled last is held: an older `serial` answers `TT_EMPTY`.
/// `TT_INVALID` for an unknown part, a `NULL` `len`, or a `NULL` `buf` with
/// a `cap`.
///
/// # Safety
///
/// `term` is `NULL` or live; `buf` is `NULL` or points to `cap` writable
/// bytes; `len` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_event_text(
    term: *const tt_term,
    serial: u64,
    part: u32,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if !out_ok(buf, cap, len) {
            return tt_status::TT_INVALID;
        }
        let core = t.core().lock();
        let polled = &core.polled;
        let text = match part {
            TT_EVENT_TEXT_BODY => &polled.body,
            TT_EVENT_TEXT_TITLE => &polled.title,
            _ => return tt_status::TT_INVALID,
        };
        if serial == 0 || serial != polled.serial {
            return tt_status::TT_EMPTY;
        }
        // SAFETY: the caller's contract, checked by `out_ok`.
        unsafe { copy_out(text, buf, cap, len) }
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Copies the URI of link `id` (from `TT_EVENT_LINK`) into `buf`, sized
/// as for `tt_term_event_text`. An id stays valid while text on the screen
/// or in history carries it; `TT_EMPTY` once it is gone, after which the
/// core may hand the id to a new link.
///
/// # Safety
///
/// As for `tt_term_event_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_link_uri(
    term: *const tt_term,
    id: u32,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if !out_ok(buf, cap, len) {
            return tt_status::TT_INVALID;
        }
        let core = t.core().lock();
        let Some(uri) = core.term.link_uri(id) else {
            return tt_status::TT_EMPTY;
        };
        // SAFETY: the caller's contract, checked by `out_ok`.
        unsafe { copy_out(uri.as_bytes(), buf, cap, len) }
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Hands the terminal's replies to the child now: the program waits for
/// its answer, and the PTY thread only collects replies after output.
/// Under the terminal lock, as the PTY thread does it, so replies keep
/// their order; the write never blocks. Bytes still waiting from before
/// go first, and new ones only once they are gone, so the buffer stays
/// within one take of the terminal's capped reply queue.
fn send_replies(t: &tt_term, core: &mut Core) {
    let Some(pty) = t.pty() else {
        return;
    };
    if core.replies.is_empty() {
        core.replies = core.term.take_replies();
    }
    if let Ok(taken) = pty.write_input(&core.replies) {
        core.replies.drain(..taken.min(core.replies.len()));
    }
}

/// Answers the OSC 52 read `id` with `len` bytes of clipboard content,
/// which the core base64-encodes for the program. `TT_INVALID` when `id`
/// is not an open read (unknown, or answered already), when the content is
/// over 1 MiB, or for `NULL` `data` with a `len`; `TT_FULL` when the reply
/// did not fit and the program got an empty answer instead.
///
/// # Safety
///
/// `term` is `NULL` or live; `data` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_clipboard_reply(
    term: *const tt_term,
    id: u64,
    data: *const u8,
    len: usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if data.is_null() && len > 0 {
            return tt_status::TT_INVALID;
        }
        let data = if len == 0 {
            &[]
        } else {
            // SAFETY: `len` readable bytes at `data`, by the caller's contract.
            unsafe { slice::from_raw_parts(data, len) }
        };
        let mut core = t.core().lock();
        let status = match core.term.clipboard_reply(id, data) {
            None => return tt_status::TT_INVALID,
            Some(true) => tt_status::TT_OK,
            Some(false) => tt_status::TT_FULL,
        };
        send_replies(t, &mut core);
        status
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Refuses the OSC 52 read `id`: the program gets an empty answer, so it
/// does not wait. `TT_INVALID` when `id` is not an open read.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_clipboard_deny(term: *const tt_term, id: u64) -> tt_status {
    let body = |t: &tt_term| {
        let mut core = t.core().lock();
        if !core.term.clipboard_deny(id) {
            return tt_status::TT_INVALID;
        }
        send_replies(t, &mut core);
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use scull_pty::{ExitStatus, Sink};

    use super::*;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    fn poll(term: *const tt_term) -> (tt_status, tt_event) {
        let mut event: tt_event = zeroed();
        event.struct_size = u32::try_from(size_of::<tt_event>()).unwrap();
        // SAFETY: live handle and a whole event.
        let status = unsafe { tt_term_poll_event(term, &mut event) };
        (status, event)
    }

    fn text(term: *const tt_term, serial: u64, part: u32, cap: usize) -> (tt_status, Vec<u8>) {
        let mut buf = vec![0xAA; cap];
        let mut len = usize::MAX;
        // SAFETY: a buffer of `cap` bytes and a writable length.
        let status =
            unsafe { tt_term_event_text(term, serial, part, buf.as_mut_ptr(), cap, &mut len) };
        if status == tt_status::TT_OK {
            buf.truncate(len);
        }
        (status, buf)
    }

    fn body(term: *const tt_term, event: &tt_event) -> String {
        let (status, bytes) = text(term, event.serial, TT_EVENT_TEXT_BODY, 4096);
        assert_eq!(status, tt_status::TT_OK);
        assert_eq!(bytes.len(), event.text_len as usize);
        String::from_utf8(bytes).unwrap()
    }

    fn link_uri(term: *const tt_term, id: u32) -> (tt_status, String) {
        let mut buf = [0u8; 64];
        let mut len = 0;
        // SAFETY: a whole buffer and a writable length.
        let status = unsafe { tt_term_link_uri(term, id, buf.as_mut_ptr(), buf.len(), &mut len) };
        let uri = String::from_utf8_lossy(&buf[..len.min(buf.len())]).into_owned();
        (status, uri)
    }

    #[test]
    fn bells_coalesce_and_the_exit_comes_once() {
        let term = new_term(10, 3);
        assert_eq!(poll(term).0, tt_status::TT_EMPTY);
        feed(term, b"\x07a\x07");
        let (status, event) = poll(term);
        assert_eq!((status, event.kind), (tt_status::TT_OK, TT_EVENT_BELL));
        assert_eq!(poll(term).0, tt_status::TT_EMPTY);
        // SAFETY: live handle; the PTY thread would make this call.
        unsafe { &*term }.core().lock().child_exited(ExitStatus {
            code: 1,
            signal: Some("SIGHUP".into()),
        });
        let (status, event) = poll(term);
        assert_eq!(status, tt_status::TT_OK);
        assert_eq!(
            (event.kind, event.exit_code, event.signaled),
            (TT_EVENT_CHILD_EXITED, 1, 1)
        );
        assert_eq!(poll(term).0, tt_status::TT_EMPTY);
        // SAFETY: NULL is refused, not written; then the live handle is freed.
        unsafe {
            assert_eq!(
                tt_term_poll_event(term, ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_poll_event(ptr::null(), ptr::null_mut()),
                tt_status::TT_INVALID
            );
            tt_term_free(term);
        }
    }

    #[test]
    fn every_kind_carries_its_details_and_text() {
        let term = new_term(10, 3);
        feed(
            term,
            "\x1b]2;\u{4e2d} title\x07\x1b]7;file://h/tmp\x07\x1b]133;D;1\x07\
             \x1b]52;p;aGk=\x07\x1b]52;c;?\x07\x1b]8;;http://x\x1b\\a\x1b]8;;\x1b\\\
             \x1b]777;notify;make;done\x07"
                .as_bytes(),
        );
        let (_, title) = poll(term);
        assert_eq!(
            (title.kind, title.detail),
            (TT_EVENT_TITLE, TT_TITLE_WINDOW)
        );
        assert_eq!(body(term, &title), "\u{4e2d} title");
        let (_, cwd) = poll(term);
        assert_eq!(cwd.kind, TT_EVENT_WORKING_DIRECTORY);
        assert_eq!(body(term, &cwd), "file://h/tmp");
        let (_, mark) = poll(term);
        assert_eq!(
            (mark.kind, mark.detail),
            (TT_EVENT_SHELL_MARK, u32::from(b'D'))
        );
        assert_eq!(body(term, &mark), "1");
        let (_, write) = poll(term);
        assert_eq!(
            (write.kind, write.detail),
            (TT_EVENT_CLIPBOARD_WRITE, u32::from(b'p'))
        );
        assert_eq!(body(term, &write), "hi");
        let (_, read) = poll(term);
        assert_eq!((read.kind, read.text_len), (TT_EVENT_CLIPBOARD_READ, 0));
        assert_ne!(read.id, 0);
        let (_, link) = poll(term);
        assert_eq!((link.kind, link.id), (TT_EVENT_LINK, 1));
        assert_eq!(body(term, &link), "http://x");
        let (_, note) = poll(term);
        assert_eq!((note.kind, note.title_len), (TT_EVENT_NOTIFICATION, 4));
        assert_eq!(body(term, &note), "done");
        let (status, title) = text(term, note.serial, TT_EVENT_TEXT_TITLE, 4);
        assert_eq!((status, title.as_slice()), (tt_status::TT_OK, &b"make"[..]));
        assert_eq!(poll(term).0, tt_status::TT_EMPTY);
        assert_eq!(link_uri(term, 1), (tt_status::TT_OK, "http://x".into()));
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn event_text_is_sized_and_only_for_the_last_poll() {
        let term = new_term(10, 3);
        feed(term, b"\x1b]0;abcdef\x07\x1b]0;x\x07");
        let (_, first) = poll(term);
        let (status, buf) = text(term, first.serial, TT_EVENT_TEXT_BODY, 5);
        assert_eq!(status, tt_status::TT_FULL);
        assert!(
            buf.iter().all(|&b| b == 0xAA),
            "a short buffer is left alone"
        );
        let mut len = 0;
        // SAFETY: a NULL buffer with no capacity asks for the length.
        let status = unsafe {
            tt_term_event_text(
                term,
                first.serial,
                TT_EVENT_TEXT_BODY,
                ptr::null_mut(),
                0,
                &mut len,
            )
        };
        assert_eq!((status, len), (tt_status::TT_FULL, 6));
        assert_eq!(text(term, first.serial, 7, 8).0, tt_status::TT_INVALID);
        assert_eq!(text(term, 0, TT_EVENT_TEXT_BODY, 8).0, tt_status::TT_EMPTY);
        let (_, second) = poll(term);
        assert_ne!(second.serial, first.serial);
        assert_eq!(
            text(term, first.serial, TT_EVENT_TEXT_BODY, 8).0,
            tt_status::TT_EMPTY,
            "a later poll replaces the text"
        );
        assert_eq!(body(term, &second), "x");
        // SAFETY: each bad pointer is refused before any access.
        unsafe {
            assert_eq!(
                tt_term_event_text(term, second.serial, 0, ptr::null_mut(), 4, &mut len),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_event_text(term, second.serial, 0, ptr::null_mut(), 0, ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_event_text(ptr::null(), 1, 0, ptr::null_mut(), 0, &mut len),
                tt_status::TT_INVALID
            );
        }
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn a_clipboard_read_is_answered_or_denied_once() {
        let term = new_term(10, 3);
        feed(term, b"\x1b]52;c;?\x07\x1b]52;c;?\x1b\\");
        let (_, first) = poll(term);
        let (_, second) = poll(term);
        // SAFETY: live handle and readable data.
        unsafe {
            assert_eq!(
                tt_term_clipboard_reply(term, first.id, b"hi".as_ptr(), 2),
                tt_status::TT_OK
            );
            assert_eq!(
                tt_term_clipboard_reply(term, first.id, b"hi".as_ptr(), 2),
                tt_status::TT_INVALID,
                "answered already"
            );
            assert_eq!(
                tt_term_clipboard_reply(term, second.id, ptr::null(), 1),
                tt_status::TT_INVALID
            );
            let big = vec![b'a'; (1 << 20) + 1];
            assert_eq!(
                tt_term_clipboard_reply(term, second.id, big.as_ptr(), big.len()),
                tt_status::TT_INVALID,
                "over the cap"
            );
            assert_eq!(tt_term_clipboard_deny(term, second.id), tt_status::TT_OK);
            assert_eq!(
                tt_term_clipboard_deny(term, second.id),
                tt_status::TT_INVALID
            );
            assert_eq!(tt_term_clipboard_deny(term, 0), tt_status::TT_INVALID);
            assert_eq!(
                tt_term_clipboard_deny(ptr::null(), 1),
                tt_status::TT_INVALID
            );
        }
        // No child: the answers wait in the terminal, in order.
        // SAFETY: live handle.
        let replies = unsafe { &*term }.core().lock().term.take_replies();
        assert_eq!(replies, b"\x1b]52;c;aGk=\x07\x1b]52;c;\x1b\\");
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn a_link_id_resolves_while_text_holds_it() {
        let term = new_term(4, 1);
        feed(term, b"\x1b]8;;http://a\x1b\\ab\x1b]8;;\x1b\\");
        let (_, link) = poll(term);
        let id = u32::try_from(link.id).unwrap();
        assert_eq!(link_uri(term, id), (tt_status::TT_OK, "http://a".into()));
        let mut len = 0;
        let mut short = [0u8; 3];
        // SAFETY: a buffer of three bytes and a writable length.
        let status = unsafe { tt_term_link_uri(term, id, short.as_mut_ptr(), 3, &mut len) };
        assert_eq!((status, len), (tt_status::TT_FULL, 8));
        assert_eq!(link_uri(term, 0).0, tt_status::TT_EMPTY);
        assert_eq!(link_uri(term, 99).0, tt_status::TT_EMPTY);
        // SAFETY: bad pointers are refused before any access.
        unsafe {
            assert_eq!(
                tt_term_link_uri(term, id, ptr::null_mut(), 0, ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_link_uri(ptr::null(), id, ptr::null_mut(), 0, &mut len),
                tt_status::TT_INVALID
            );
            (*term).poison();
        }
        assert_eq!(link_uri(term, id).0, tt_status::TT_POISONED);
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }
}
