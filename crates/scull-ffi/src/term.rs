//! The terminal handle: the terminal state behind the one lock that the
//! frame reads and the PTY thread share, and the poison flag a caught panic
//! sets. Separate from the frame because the frame is the UI's and this is
//! the core's.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use scull_pty::Mutex;
use scull_term::Terminal;

use crate::abi_compatible;
use crate::guard::{SizedStruct, guard, read_sized, tt_status, with_term};

/// Most cells one screen may have. The size comes from the host, and a
/// bad one must be refused rather than abort the process on allocation:
/// 4 Mi cells is 32 MiB per screen, past any display's need.
const MAX_SCREEN_CELLS: usize = 1 << 22;

/// How to create a terminal. Fields a host does not know read as zero.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_term_options {
    /// `sizeof(tt_term_options)` as the host knows it.
    pub struct_size: u32,
    /// `TT_ABI_VERSION` as the host was built with it.
    pub abi_version: u32,
    /// Columns, at least 1.
    pub cols: u16,
    /// Rows, at least 1.
    pub rows: u16,
    /// History rows to keep; the core clamps it.
    pub scrollback: u32,
}

// SAFETY: repr(C), `struct_size` first, integers only.
unsafe impl SizedStruct for tt_term_options {}

/// What the terminal lock guards.
pub(crate) struct Core {
    pub(crate) term: Terminal,
}

/// One terminal. Opaque to the host.
pub struct tt_term {
    core: Arc<Mutex<Core>>,
    poisoned: AtomicBool,
}

impl tt_term {
    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }

    /// Marks the terminal unusable after a caught panic: its state may be
    /// half-updated, so nothing reads it again.
    pub(crate) fn poison(&self) {
        self.poisoned.store(true, Ordering::Release);
    }

    pub(crate) fn core(&self) -> &Mutex<Core> {
        &self.core
    }
}

/// Whether a `cols` x `rows` screen is one the core accepts.
fn size_ok(cols: u16, rows: u16) -> bool {
    cols > 0 && rows > 0 && usize::from(cols) * usize::from(rows) <= MAX_SCREEN_CELLS
}

/// Creates a terminal with no child process: bytes reach it only through
/// `tt_term_feed`. On `TT_OK` `*out` holds the handle; free it with
/// `tt_term_free`.
///
/// # Safety
///
/// `options` is `NULL` or points to `struct_size` readable bytes; `out` is
/// `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_new(
    options: *const tt_term_options,
    out: *mut *mut tt_term,
) -> tt_status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(options) = (unsafe { read_sized(options) }) else {
            return tt_status::TT_INVALID;
        };
        if out.is_null() {
            return tt_status::TT_INVALID;
        }
        if !abi_compatible(options.abi_version) {
            return tt_status::TT_ABI_MISMATCH;
        }
        if !size_ok(options.cols, options.rows) {
            return tt_status::TT_INVALID;
        }
        let scrollback = usize::try_from(options.scrollback).unwrap_or(usize::MAX);
        let Ok(term) = Terminal::new(options.cols, options.rows, scrollback) else {
            return tt_status::TT_INVALID;
        };
        let handle = Box::new(tt_term {
            core: Arc::new(Mutex::new(Core { term })),
            poisoned: AtomicBool::new(false),
        });
        // SAFETY: `out` is non-null and writable by the caller's contract.
        unsafe { out.write(Box::into_raw(handle)) };
        tt_status::TT_OK
    })
}

/// Parses `len` bytes of program output into the terminal, as if the
/// child had written them. `bytes` may be `NULL` when `len` is 0.
///
/// # Safety
///
/// `term` is `NULL` or live; `bytes` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_feed(
    term: *const tt_term,
    bytes: *const u8,
    len: usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if len == 0 {
            return tt_status::TT_OK;
        }
        if bytes.is_null() {
            return tt_status::TT_INVALID;
        }
        // SAFETY: `len` readable bytes at `bytes`, by the caller's contract.
        let bytes = unsafe { std::slice::from_raw_parts(bytes, len) };
        t.core().lock().term.feed(bytes);
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Frees the terminal. Also valid on a poisoned terminal; `NULL` is a
/// no-op. No other call may use `term` during or after this one.
///
/// # Safety
///
/// `term` is `NULL` or live, and not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_free(term: *mut tt_term) {
    if term.is_null() {
        return;
    }
    // A panic while dropping leaks what is left rather than abort the host.
    let _ = guard(|| {
        // SAFETY: from `Box::into_raw` in `tt_term_new`, freed only here.
        drop(unsafe { Box::from_raw(term) });
        tt_status::TT_OK
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use std::ptr;

    use super::*;
    use crate::TT_ABI_VERSION;

    pub(crate) fn options(cols: u16, rows: u16) -> tt_term_options {
        tt_term_options {
            struct_size: u32::try_from(size_of::<tt_term_options>()).unwrap(),
            abi_version: TT_ABI_VERSION,
            cols,
            rows,
            scrollback: 100,
        }
    }

    /// A live terminal, freed by the caller.
    pub(crate) fn new_term(cols: u16, rows: u16) -> *mut tt_term {
        let mut term = ptr::null_mut();
        // SAFETY: valid options and out pointer.
        let status = unsafe { tt_term_new(&options(cols, rows), &mut term) };
        assert_eq!(status, tt_status::TT_OK);
        term
    }

    pub(crate) fn feed(term: *const tt_term, bytes: &[u8]) -> tt_status {
        // SAFETY: `bytes` is readable for its length.
        unsafe { tt_term_feed(term, bytes.as_ptr(), bytes.len()) }
    }

    fn new_with(options: &tt_term_options) -> tt_status {
        let mut term = ptr::null_mut();
        // SAFETY: valid pointers.
        let status = unsafe { tt_term_new(options, &mut term) };
        // SAFETY: NULL or the handle just made.
        unsafe { tt_term_free(term) };
        status
    }

    #[test]
    fn creation_checks_its_arguments() {
        assert_eq!(new_with(&options(80, 24)), tt_status::TT_OK);
        assert_eq!(new_with(&options(0, 24)), tt_status::TT_INVALID);
        assert_eq!(
            new_with(&options(u16::MAX, u16::MAX)),
            tt_status::TT_INVALID
        );
        let other_abi = tt_term_options {
            abi_version: TT_ABI_VERSION + 1,
            ..options(80, 24)
        };
        assert_eq!(new_with(&other_abi), tt_status::TT_ABI_MISMATCH);
        let no_abi = tt_term_options {
            struct_size: 4,
            ..options(80, 24)
        };
        assert_eq!(
            new_with(&no_abi),
            tt_status::TT_ABI_MISMATCH,
            "missing fields read as zero"
        );
        // SAFETY: NULL pointers are allowed.
        unsafe {
            assert_eq!(
                tt_term_new(ptr::null(), &mut ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_new(&options(80, 24), ptr::null_mut()),
                tt_status::TT_INVALID
            );
        }
    }

    #[test]
    fn a_null_terminal_is_invalid_everywhere() {
        assert_eq!(feed(ptr::null(), b"x"), tt_status::TT_INVALID);
        // SAFETY: NULL is allowed.
        unsafe { tt_term_free(ptr::null_mut()) };
    }

    #[test]
    fn feed_reaches_the_terminal() {
        let term = new_term(10, 3);
        assert_eq!(feed(term, b"abcdefgh"), tt_status::TT_OK);
        assert_eq!(feed(term, &[]), tt_status::TT_OK);
        // SAFETY: NULL bytes with a length is refused, not read.
        assert_eq!(
            unsafe { tt_term_feed(term, ptr::null(), 1) },
            tt_status::TT_INVALID
        );
        // SAFETY: live handle.
        unsafe {
            let core = (*term).core().lock();
            assert_eq!((core.term.cursor().row, core.term.cursor().col), (0, 8));
        }
        // SAFETY: live handle, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn a_panic_poisons_only_its_own_terminal() {
        let (sick, well) = (new_term(10, 3), new_term(10, 3));
        // SAFETY: live handle.
        let status = unsafe { with_term(sick, |_| panic!("core bug")) };
        assert_eq!(status, tt_status::TT_PANIC);
        assert_eq!(feed(sick, b"x"), tt_status::TT_POISONED);
        // SAFETY: live handles.
        unsafe {
            assert_eq!(feed(well, b"x"), tt_status::TT_OK);
            tt_term_free(sick);
            tt_term_free(well);
        }
    }
}
