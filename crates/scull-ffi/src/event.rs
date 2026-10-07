//! The polled event queue: what happened besides the picture changing.
//! Polled rather than called back so the host handles events on its own
//! thread, with no core lock held and no reentrancy into the core.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use scull_pty::limits::UNKNOWN_EXIT_CODE;

use crate::guard::{SizedStruct, can_write, tt_status, with_term, write_sized, zeroed};
use crate::term::tt_term;

/// `tt_event.kind`: the program rang the bell, once or more since the last
/// poll.
pub const TT_EVENT_BELL: u32 = 1;

/// `tt_event.kind`: the child ended and all its output has been parsed.
/// Reported once, last.
pub const TT_EVENT_CHILD_EXITED: u32 = 2;

/// `tt_event.exit_code` when the system did not say. A literal, not the
/// PTY crate's constant, so the generated header can spell it.
pub const TT_EXIT_CODE_UNKNOWN: u32 = 0xFFFF_FFFF;

const _: () = assert!(TT_EXIT_CODE_UNKNOWN == UNKNOWN_EXIT_CODE);

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
}

// SAFETY: repr(C), `struct_size` first, integers only.
unsafe impl SizedStruct for tt_event {}

/// Takes the next event into `*event`: `TT_OK` with one, `TT_EMPTY` when
/// there is none. Call it until `TT_EMPTY` after every wakeup.
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
        let mut next: tt_event = zeroed();
        {
            let mut core = t.core().lock();
            if core.term.take_bell() {
                next.kind = TT_EVENT_BELL;
            } else if let Some(exit) = core.exit.take() {
                next.kind = TT_EVENT_CHILD_EXITED;
                next.exit_code = exit.code;
                next.signaled = u8::from(exit.signal.is_some());
            } else {
                return tt_status::TT_EMPTY;
            }
        }
        // SAFETY: checked writable above.
        unsafe { write_sized(event, &next) };
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
}
