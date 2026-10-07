//! Reading the screen as text, for a host's screen reader support
//! (NSAccessibility, UI Automation) and copying. A cold path: the text is
//! built under the terminal lock and copied out into the host's buffer, so
//! the host frees nothing.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use crate::guard::{tt_status, with_term};
use crate::term::tt_term;

/// Copies the text of `rows` viewport rows from `row` into `buf`: UTF-8,
/// one line per row joined by `\n` (line N is row `row + N`), wide
/// characters once, blank and concealed (SGR 8) cells as spaces, trailing
/// spaces trimmed, no NUL. `*len` becomes the text's length. When that is
/// more than `cap`, nothing is copied and the answer is `TT_FULL`: call
/// again with a larger buffer (pass `cap` 0 to ask for the length). Rows
/// past the viewport are not read. `TT_INVALID` for a `NULL` `len`, or a
/// `NULL` `buf` with a `cap`.
///
/// # Safety
///
/// `term` is `NULL` or live; `buf` is `NULL` or points to `cap` writable
/// bytes; `len` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_read_text(
    term: *const tt_term,
    row: u16,
    rows: u16,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if len.is_null() || (buf.is_null() && cap > 0) {
            return tt_status::TT_INVALID;
        }
        let mut text = String::new();
        let end = row.saturating_add(rows);
        t.core().lock().term.read_text(row..end, &mut text);
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { len.write(text.len()) };
        if text.len() > cap {
            return tt_status::TT_FULL;
        }
        if !text.is_empty() {
            // SAFETY: `buf` holds `cap` writable bytes, at least the text's
            // length, and cannot overlap a String the core just built.
            unsafe { buf.copy_from_nonoverlapping(text.as_ptr(), text.len()) };
        }
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    fn read(term: *const tt_term, row: u16, rows: u16, cap: usize) -> (tt_status, usize, Vec<u8>) {
        let mut buf = vec![0xAA; cap];
        let mut len = usize::MAX;
        // SAFETY: a buffer of `cap` bytes and a writable length.
        let status = unsafe { tt_term_read_text(term, row, rows, buf.as_mut_ptr(), cap, &mut len) };
        (status, len, buf)
    }

    #[test]
    fn the_text_fits_or_its_length_is_answered() {
        let term = new_term(6, 3);
        feed(term, "ab\u{4e2d}\r\n\x1b[8mpw\x1b[m!".as_bytes());
        let text = "ab\u{4e2d}\n  !\n"; // the third row is empty
        let (status, len, buf) = read(term, 0, u16::MAX, 64);
        assert_eq!((status, len), (tt_status::TT_OK, text.len()));
        assert_eq!(&buf[..len], text.as_bytes());
        assert_eq!(buf[len], 0xAA, "nothing past the text is written");
        let (status, len, buf) = read(term, 0, 3, text.len() - 1);
        assert_eq!((status, len), (tt_status::TT_FULL, text.len()));
        assert!(
            buf.iter().all(|&b| b == 0xAA),
            "a short buffer is left alone"
        );
        let mut len = 0;
        // SAFETY: a NULL buffer with no capacity asks for the length.
        let status = unsafe { tt_term_read_text(term, 1, 1, ptr::null_mut(), 0, &mut len) };
        assert_eq!((status, len), (tt_status::TT_FULL, "  !".len()));
        assert_eq!(
            read(term, 9, 1, 4).0,
            tt_status::TT_OK,
            "past the viewport reads empty"
        );
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn bad_arguments_and_poisoned_terminals_are_refused() {
        let term = new_term(4, 2);
        let mut len = 0;
        // SAFETY: each NULL is refused before any access.
        unsafe {
            assert_eq!(
                tt_term_read_text(term, 0, 1, ptr::null_mut(), 0, ptr::null_mut()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_read_text(term, 0, 1, ptr::null_mut(), 4, &mut len),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_term_read_text(ptr::null(), 0, 1, ptr::null_mut(), 0, &mut len),
                tt_status::TT_INVALID
            );
            (*term).poison();
        }
        assert_eq!(read(term, 0, 1, 4).0, tt_status::TT_POISONED);
        // SAFETY: live, not used again.
        unsafe { tt_term_free(term) };
    }
}
