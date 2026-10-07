//! Terminals with a child: the host's strings turned into spawn options,
//! the terminal state plugged into the PTY threads as their sink, the
//! wakeup that tells the host to look, and input to the child. Separate
//! from `term.rs` because everything here runs on, or talks to, the PTY
//! threads.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use scull_pty::{ExitStatus, Mutex, Pty, PtyError, PtySize, Sink, SpawnOptions};

use crate::guard::{guard, tt_status, with_term};
use crate::term::{Core, create, hand_out, tt_term, tt_term_options};

/// UTF-8 text the host owns: `len` bytes at `ptr`, no terminator. `ptr`
/// may be `NULL` when `len` is 0.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_str {
    /// The first byte.
    pub ptr: *const u8,
    /// Bytes, not characters.
    pub len: usize,
}

/// Called with the `userdata` given at spawn when the terminal has
/// something new: output to draw or an event to poll. It runs on a core
/// thread and must not block, unwind or call any `tt_*` function: post a
/// redraw to the UI thread and return. It is not called again until the
/// host calls `tt_frame_update` or `tt_term_poll_event` on the terminal,
/// and never after `tt_term_free` returns.
pub type tt_wakeup_fn = Option<unsafe extern "C" fn(userdata: *mut c_void)>;

/// The host's wakeup, coalesced: a burst of output posts one redraw.
pub(crate) struct Wake {
    func: unsafe extern "C" fn(*mut c_void),
    userdata: *mut c_void,
    /// Set by a wakeup, cleared when the host looks.
    pending: AtomicBool,
}

// SAFETY: the host promises, by giving the callback, that it may be called
// with this userdata from any thread (`tt_wakeup_fn`).
unsafe impl Send for Wake {}
// SAFETY: as for `Send`; the only shared state is the atomic.
unsafe impl Sync for Wake {}

impl Wake {
    fn fire(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        // SAFETY: the host's callback with the host's userdata, under the
        // `tt_wakeup_fn` contract.
        unsafe { (self.func)(self.userdata) };
    }

    pub(crate) fn rearm(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

/// The text behind `s`, or `None` for `NULL` with a length or bytes that
/// are not UTF-8.
///
/// # Safety
///
/// `s.ptr` points to `s.len` readable bytes, or `s.len` is 0.
pub(crate) unsafe fn text<'a>(s: tt_str) -> Option<&'a str> {
    if s.len == 0 {
        return Some("");
    }
    if s.ptr.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    std::str::from_utf8(unsafe { slice::from_raw_parts(s.ptr, s.len) }).ok()
}

/// The `len` strings at `ptr`, each checked by [`text`].
///
/// # Safety
///
/// `ptr` points to `len` readable `tt_str`s, each valid for [`text`], or
/// `len` is 0.
unsafe fn texts<'a>(ptr: *const tt_str, len: usize) -> Option<Vec<&'a str>> {
    if len == 0 {
        return Some(Vec::new());
    }
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let all = unsafe { slice::from_raw_parts(ptr, len) };
    // SAFETY: the caller's contract, for each element.
    all.iter().map(|&s| unsafe { text(s) }).collect()
}

/// What to run, from the host's options; `None` when a string is invalid.
///
/// # Safety
///
/// The strings and arrays in `o` are valid as for [`texts`].
unsafe fn spawn_options(o: &tt_term_options) -> Option<SpawnOptions> {
    // SAFETY: the caller's contract, for every string below.
    let (program, args, cwd, env) = unsafe {
        (
            text(o.program)?,
            texts(o.args, o.args_len)?,
            text(o.cwd)?,
            texts(o.env, o.env_len)?,
        )
    };
    let env = env
        .into_iter()
        .map(|entry| match entry.split_once('=') {
            Some((name, value)) if !name.is_empty() => Some((name.into(), value.into())),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(SpawnOptions {
        program: (!program.is_empty()).then(|| program.into()),
        args: args.into_iter().map(Into::into).collect(),
        cwd: (!cwd.is_empty()).then(|| cwd.into()),
        env,
        size: PtySize {
            rows: o.rows,
            cols: o.cols,
            pixel_width: 0,
            pixel_height: 0,
        },
    })
}

impl Core {
    /// Runs `body` unless the terminal is poisoned, and poisons it if
    /// `body` panics: a PTY thread has no caller to answer `TT_PANIC` to,
    /// so the next call on the handle answers `TT_POISONED` instead.
    fn guarded(&mut self, body: impl FnOnce(&mut Self)) {
        if self.poisoned.load(Ordering::Acquire) {
            return;
        }
        if catch_unwind(AssertUnwindSafe(|| body(self))).is_err() {
            self.poisoned.store(true, Ordering::Release);
        }
    }
}

impl Sink for Core {
    fn feed(&mut self, bytes: &[u8]) {
        // A poisoned terminal drops output rather than stall the child.
        self.guarded(|core| core.term.feed(bytes));
    }

    fn drain_replies(&mut self, out: &mut Vec<u8>, limit: usize) {
        self.guarded(|core| {
            if core.replies.is_empty() {
                core.replies = core.term.take_replies();
            }
            let take = core.replies.len().min(limit);
            out.extend(core.replies.drain(..take));
        });
    }

    fn child_exited(&mut self, status: ExitStatus) {
        self.exit = Some(status);
    }

    fn wants_wakeup(&mut self, now: Instant) -> bool {
        let mut held = false;
        self.guarded(|core| held = core.term.sync_held(now));
        // A poisoned terminal wakes the host so it learns from the next call.
        !held
    }

    fn deadline(&self) -> Option<Instant> {
        // A poisoned terminal holds nothing; a deadline would only spin.
        if self.poisoned.load(Ordering::Acquire) {
            return None;
        }
        self.term.sync_deadline()
    }
}

/// Creates a terminal running a child on a new PTY: `options.program`
/// with its arguments, or the user's shell. Output is parsed on core
/// threads; `options.wakeup` says when to look. On `TT_OK` `*out` holds
/// the handle; free it with `tt_term_free`, which ends the child.
///
/// # Safety
///
/// As for `tt_term_new`; the strings and arrays in `*options` are valid
/// for their lengths, during this call only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_spawn(
    options: *const tt_term_options,
    out: *mut *mut tt_term,
) -> tt_status {
    guard(|| {
        if out.is_null() {
            return tt_status::TT_INVALID;
        }
        // SAFETY: the caller's contract.
        let (options, core) = match unsafe { create(options) } {
            Ok(made) => made,
            Err(status) => return status,
        };
        // SAFETY: the caller's contract.
        let Some(spawn) = (unsafe { spawn_options(&options) }) else {
            return tt_status::TT_INVALID;
        };
        let wake = options.wakeup.map(|func| {
            Arc::new(Wake {
                func,
                userdata: options.userdata,
                pending: AtomicBool::new(false),
            })
        });
        let fire = wake.clone();
        let core = Arc::new(Mutex::new(core));
        let wakeup = move || {
            if let Some(wake) = &fire {
                wake.fire();
            }
        };
        let Ok(pty) = Pty::spawn(&spawn, Arc::clone(&core), wakeup) else {
            return tt_status::TT_IO;
        };
        // SAFETY: `out` is non-null and writable by the caller's contract.
        unsafe { hand_out(out, tt_term::new(core, Some(pty), wake)) }
    })
}

/// Queues `len` bytes of input (keys, paste) for the child without
/// blocking. `*written` gets how many leading bytes were taken: fewer than
/// `len` when the child is not reading; offer the rest later. `TT_CLOSED`
/// when there is no child or it has gone.
///
/// # Safety
///
/// `term` is `NULL` or live; `bytes` points to `len` readable bytes;
/// `written` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_write(
    term: *const tt_term,
    bytes: *const u8,
    len: usize,
    written: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if written.is_null() || (bytes.is_null() && len > 0) {
            return tt_status::TT_INVALID;
        }
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { written.write(0) };
        let Some(pty) = t.pty() else {
            return tt_status::TT_CLOSED;
        };
        let bytes = if len == 0 {
            &[]
        } else {
            // SAFETY: `len` readable bytes at `bytes`, by the caller's contract.
            unsafe { slice::from_raw_parts(bytes, len) }
        };
        match pty.write_input(bytes) {
            Ok(taken) => {
                // SAFETY: as above.
                unsafe { written.write(taken) };
                tt_status::TT_OK
            }
            Err(PtyError::Closed) => tt_status::TT_CLOSED,
            Err(_) => tt_status::TT_IO,
        }
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Starts an interactive resize: the child's output is held back, so the
/// host keeps drawing the last frame, until `tt_term_resize` with the final
/// size. Without it every intermediate size would rewrap the history.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_resize_begin(term: *const tt_term) -> tt_status {
    let body = |t: &tt_term| {
        if let Some(pty) = t.pty() {
            pty.pause_output();
        }
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    #[test]
    fn a_panic_on_a_pty_thread_poisons_the_handle() {
        let term = new_term(10, 3);
        {
            // SAFETY: live handle.
            let mut core = unsafe { &*term }.core().lock();
            core.guarded(|_| panic!("core bug"));
            // What the PTY thread does next must neither touch the state
            // nor keep the host from hearing about it.
            core.feed(b"x");
            assert!(core.wants_wakeup(Instant::now()));
            assert_eq!(core.deadline(), None);
        }
        assert_eq!(feed(term, b"x"), tt_status::TT_POISONED);
        // SAFETY: live handle, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn replies_go_out_in_pieces_no_larger_than_asked() {
        let term = new_term(10, 3);
        // Two device attribute requests queue two replies.
        feed(term, b"\x1b[c\x1b[c");
        // SAFETY: live handle.
        let mut core = unsafe { &*term }.core().lock();
        let (mut out, piece) = (Vec::new(), 3);
        core.drain_replies(&mut out, piece);
        assert_eq!(out.len(), piece);
        core.drain_replies(&mut out, usize::MAX);
        assert!(core.replies.is_empty());
        let one = out.len() / 2;
        assert!(
            out.starts_with(b"\x1b[?") && out[..one] == out[one..],
            "{out:?}"
        );
        drop(core);
        // SAFETY: live handle, not used again.
        unsafe { tt_term_free(term) };
    }
}
