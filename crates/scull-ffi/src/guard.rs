//! Status codes, the panic guard every export runs its body in, and the
//! `struct_size` rule for structs that cross the boundary. One module so no
//! export can forget a check: unwinding out of an `extern "C"` function
//! aborts the whole host, every tab with it.
#![allow(unsafe_code)] // Reads and writes the caller's structs by `struct_size`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::term::tt_term;

/// What every fallible call answers. Values are only ever appended.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum tt_status {
    /// Done.
    TT_OK = 0,
    /// A `NULL` handle or pointer, a `struct_size` too small, a size of
    /// zero, or text that is not UTF-8.
    TT_INVALID = 1,
    /// The host was built against another ABI version.
    TT_ABI_MISMATCH = 2,
    /// An earlier call panicked inside this terminal; only
    /// `tt_term_free` is left to call.
    TT_POISONED = 3,
    /// This call panicked inside the core; the terminal is now poisoned.
    TT_PANIC = 4,
    /// Nothing to report: the event queue is empty.
    TT_EMPTY = 5,
    /// The terminal has no child to talk to: it was made by
    /// `tt_term_new`, or its child has gone.
    TT_CLOSED = 6,
    /// The operating system refused: no PTY, a program that does not
    /// start, a size the PTY rejects.
    TT_IO = 7,
    /// The child is not reading and its input queue cannot take the whole
    /// event; nothing was sent. Try again later.
    TT_FULL = 8,
}

/// Runs `body` with panics caught; a panic answers `TT_PANIC`.
pub(crate) fn guard(body: impl FnOnce() -> tt_status) -> tt_status {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(tt_status::TT_PANIC)
}

/// Runs `body` on the terminal behind `term`: `NULL` answers `TT_INVALID`,
/// a poisoned terminal `TT_POISONED`, and a panic poisons it.
///
/// # Safety
///
/// `term` is `NULL` or a live handle from `tt_term_new`.
pub(crate) unsafe fn with_term(
    term: *const tt_term,
    body: impl FnOnce(&tt_term) -> tt_status,
) -> tt_status {
    // SAFETY: the caller's contract: NULL or a live handle.
    let Some(term) = (unsafe { term.as_ref() }) else {
        return tt_status::TT_INVALID;
    };
    if term.is_poisoned() {
        return tt_status::TT_POISONED;
    }
    catch_unwind(AssertUnwindSafe(|| body(term))).unwrap_or_else(|_| {
        term.poison();
        tt_status::TT_PANIC
    })
}

/// A struct that crosses the boundary with `struct_size` first.
///
/// # Safety
///
/// The type is `#[repr(C)]`, starts with a `u32` `struct_size`, and every
/// field is an integer, a raw pointer or an optional function pointer, so
/// all-zero bytes are a valid value.
pub(crate) unsafe trait SizedStruct: Copy {}

/// An all-zero `T`: every field a host does not set.
pub(crate) fn zeroed<T: SizedStruct>() -> T {
    // SAFETY: `T: SizedStruct` makes all-zero a valid value.
    unsafe { std::mem::zeroed() }
}

/// Bytes of `struct_size` itself: the least a caller's struct can be.
const SIZE_FIELD: usize = size_of::<u32>();

/// Reads the `struct_size` the caller put first in `at`.
///
/// # Safety
///
/// `at` is `NULL` or points to at least four readable bytes.
unsafe fn caller_size<T: SizedStruct>(at: *const T) -> Option<usize> {
    if at.is_null() {
        return None;
    }
    // SAFETY: non-null, and the caller's struct starts with a u32.
    let size = unsafe { at.cast::<u32>().read_unaligned() };
    usize::try_from(size).ok().filter(|&s| s >= SIZE_FIELD)
}

/// The caller's struct as this library knows it: the first `struct_size`
/// bytes of it, zero after. `None` for `NULL` or a size below four.
///
/// # Safety
///
/// `src` is `NULL` or points to `struct_size` readable bytes.
pub(crate) unsafe fn read_sized<T: SizedStruct>(src: *const T) -> Option<T> {
    // SAFETY: the caller's contract.
    let size = unsafe { caller_size(src) }?;
    let mut out: T = zeroed();
    let len = size.min(size_of::<T>());
    // SAFETY: `src` has `size` readable bytes and `out` has `size_of::<T>()`,
    // both at least `len`; they cannot overlap since `out` is a local.
    unsafe { ptr::copy_nonoverlapping(src.cast::<u8>(), (&raw mut out).cast::<u8>(), len) };
    Some(out)
}

/// Whether `dst` can take an output struct: not `NULL`, `struct_size` at
/// least four. Checked before work whose result could not be handed back.
///
/// # Safety
///
/// As for [`write_sized`].
pub(crate) unsafe fn can_write<T: SizedStruct>(dst: *mut T) -> bool {
    // SAFETY: the caller's contract.
    unsafe { caller_size(dst.cast_const()) }.is_some()
}

/// Writes `value` into the caller's struct, as much of it as the caller's
/// `struct_size` holds, and sets `struct_size` to the bytes written.
/// False, writing nothing, for `NULL` or a size below four.
///
/// # Safety
///
/// `dst` is `NULL` or points to `struct_size` writable bytes.
pub(crate) unsafe fn write_sized<T: SizedStruct>(dst: *mut T, value: &T) -> bool {
    // SAFETY: the caller's contract.
    let Some(size) = (unsafe { caller_size(dst.cast_const()) }) else {
        return false;
    };
    let len = size.min(size_of::<T>());
    // SAFETY: `dst` has `size` writable bytes and `value` `size_of::<T>()`,
    // both at least `len`; a caller cannot pass a pointer into our own
    // local `value`, so they do not overlap.
    unsafe { ptr::copy_nonoverlapping(ptr::from_ref(value).cast::<u8>(), dst.cast::<u8>(), len) };
    let written = u32::try_from(len).unwrap_or(u32::MAX);
    // SAFETY: the first four bytes are the caller's `struct_size`.
    unsafe { dst.cast::<u32>().write_unaligned(written) };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct Two {
        struct_size: u32,
        a: u32,
        b: u64,
    }

    // SAFETY: repr(C), u32 first, integers only.
    unsafe impl SizedStruct for Two {}

    /// A host built against version 1 of `Two`, which had no `b`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct One {
        struct_size: u32,
        a: u32,
    }

    #[test]
    fn a_panic_becomes_a_status() {
        assert_eq!(guard(|| panic!("boom")), tt_status::TT_PANIC);
        assert_eq!(guard(|| tt_status::TT_OK), tt_status::TT_OK);
    }

    #[test]
    fn a_smaller_struct_reads_as_zero_past_its_end() {
        let old = One {
            struct_size: u32::try_from(size_of::<One>()).unwrap(),
            a: 7,
        };
        // SAFETY: `old` has struct_size readable bytes.
        let read = unsafe { read_sized(ptr::from_ref(&old).cast::<Two>()) }.unwrap();
        assert_eq!((read.a, read.b), (7, 0));
    }

    #[test]
    fn a_larger_struct_is_read_up_to_what_this_library_knows() {
        let mut bytes = [0xAAu8; 64];
        let size = u32::try_from(bytes.len()).unwrap();
        bytes[..4].copy_from_slice(&size.to_ne_bytes());
        // SAFETY: 64 readable bytes.
        let read = unsafe { read_sized(bytes.as_ptr().cast::<Two>()) }.unwrap();
        assert_eq!(read.a, u32::from_ne_bytes([0xAA; 4]));
    }

    #[test]
    fn null_and_tiny_sizes_are_refused() {
        // SAFETY: NULL is allowed.
        assert!(unsafe { read_sized::<Two>(ptr::null()) }.is_none());
        let tiny = Two {
            struct_size: 3,
            ..Two::default()
        };
        // SAFETY: a whole `Two` is readable.
        assert!(unsafe { read_sized(&raw const tiny) }.is_none());
        let mut out = tiny;
        // SAFETY: a whole `Two` is writable.
        assert!(!unsafe { write_sized(&raw mut out, &Two::default()) });
        assert!(!unsafe { can_write(&raw mut out) });
    }

    #[test]
    fn writing_stops_at_the_callers_size_and_reports_it() {
        let mut old = One {
            struct_size: u32::try_from(size_of::<One>()).unwrap(),
            a: 0,
        };
        let full = Two {
            struct_size: 0,
            a: 5,
            b: u64::MAX,
        };
        let dst = (&raw mut old).cast::<Two>();
        // SAFETY: `old` has struct_size writable bytes.
        assert!(unsafe { can_write(dst) && write_sized(dst, &full) });
        assert_eq!(old.a, 5);
        assert_eq!(usize::try_from(old.struct_size).unwrap(), size_of::<One>());
    }
}
