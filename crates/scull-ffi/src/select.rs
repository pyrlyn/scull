//! Selection and scrollback search for the host's mouse, copy and find
//! bar. The core keeps both and marks their cells in the frame
//! (`TT_CELL_SELECTED`, `TT_CELL_MATCH`, `TT_CELL_CURRENT_MATCH`); the host
//! calls `tt_frame_update` after a change to see it.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use scull_term::{
    MAX_SEARCH_MATCHES, MAX_SEARCH_PATTERN, MAX_SELECTION_BYTES, Match, SelectionKind,
};

use crate::guard::{SizedStruct, can_write, tt_status, with_term, write_sized, zeroed};
use crate::spawn::{text, tt_str};
use crate::term::tt_term;
use crate::text::{copy_out, out_ok};

/// `tt_term_select_start` kind: cell by cell, for a drag.
pub const TT_SELECT_CELL: u32 = 0;
/// Whole words, for a double click.
pub const TT_SELECT_WORD: u32 = 1;
/// Whole lines, soft wraps included, for a triple click.
pub const TT_SELECT_LINE: u32 = 2;
/// A rectangle of columns.
pub const TT_SELECT_BLOCK: u32 = 3;

/// `tt_term_search_set` flag: fold case.
pub const TT_SEARCH_IGNORE_CASE: u32 = 1;

/// Longest pattern, in bytes, `tt_term_search_set` accepts.
pub const TT_MAX_SEARCH_PATTERN: usize = 256;
/// Most matches `tt_term_search_count` counts.
pub const TT_MAX_SEARCH_MATCHES: usize = 10000;
/// Most bytes `tt_term_selection_text` hands out; longer text is cut.
pub const TT_MAX_SELECTION_BYTES: usize = 16777216;

const _: () = {
    assert!(TT_MAX_SEARCH_PATTERN == MAX_SEARCH_PATTERN);
    assert!(TT_MAX_SEARCH_MATCHES == MAX_SEARCH_MATCHES);
    assert!(TT_MAX_SELECTION_BYTES == MAX_SELECTION_BYTES);
};

/// A search match: first and last cell, inclusive. Lines are absolute
/// (counted from the first line ever output), so a match keeps its
/// numbers while the text scrolls.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_match {
    /// `sizeof(tt_match)` as the host knows it.
    pub struct_size: u32,
    /// Column of the first cell.
    pub start_col: u16,
    /// Column of the last cell (a wide character's right half included).
    pub end_col: u16,
    /// Line of the first cell.
    pub start_line: u64,
    /// Line of the last cell.
    pub end_line: u64,
}

// SAFETY: repr(C), `struct_size` first, integers only.
unsafe impl SizedStruct for tt_match {}

fn selection_kind(kind: u32) -> Option<SelectionKind> {
    match kind {
        TT_SELECT_CELL => Some(SelectionKind::Cell),
        TT_SELECT_WORD => Some(SelectionKind::Word),
        TT_SELECT_LINE => Some(SelectionKind::Line),
        TT_SELECT_BLOCK => Some(SelectionKind::Block),
        _ => None,
    }
}

/// Starts a selection of `kind` (`TT_SELECT_*`) at viewport cell `row`,
/// `col` (clamped), replacing any other. It covers that cell, word or line
/// until extended; clear it on a click that did not drag. Output that
/// rewrites its rows, a resize and a screen switch drop it. `TT_INVALID`
/// for an unknown kind.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_select_start(
    term: *const tt_term,
    kind: u32,
    row: u16,
    col: u16,
) -> tt_status {
    let body = |t: &tt_term| {
        let Some(kind) = selection_kind(kind) else {
            return tt_status::TT_INVALID;
        };
        let term = &mut t.core().lock().term;
        let at = term.viewport_point(row, col);
        term.select_start(kind, at);
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Moves the selection's free end to viewport cell `row`, `col` (clamped).
/// No selection is no change.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_select_extend(
    term: *const tt_term,
    row: u16,
    col: u16,
) -> tt_status {
    let body = |t: &tt_term| {
        let term = &mut t.core().lock().term;
        let to = term.viewport_point(row, col);
        term.select_extend(to);
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Drops the selection.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_select_clear(term: *const tt_term) -> tt_status {
    let body = |t: &tt_term| {
        t.core().lock().term.select_clear();
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Copies the selected text into `buf` as `tt_term_read_text` does:
/// UTF-8, soft-wrapped rows joined, hard line ends as `\n`, wide characters
/// once, blank and concealed cells as spaces, trailing spaces trimmed per
/// line, no NUL, at most `TT_MAX_SELECTION_BYTES`. `*len` becomes its
/// length; `TT_FULL` with nothing copied when that is more than `cap`.
/// `TT_EMPTY` with `*len` 0 when nothing is selected. `TT_INVALID` for a
/// `NULL` `len`, or a `NULL` `buf` with a `cap`.
///
/// # Safety
///
/// `term` is `NULL` or live; `buf` is `NULL` or points to `cap` writable
/// bytes; `len` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_selection_text(
    term: *const tt_term,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if !out_ok(buf, cap, len) {
            return tt_status::TT_INVALID;
        }
        let mut text = String::new();
        let selected = {
            let core = t.core().lock();
            core.term.selection_text(&mut text);
            core.term.selection().is_some()
        };
        // SAFETY: the caller's contract, checked above.
        let status = unsafe { copy_out(text.as_bytes(), buf, cap, len) };
        if selected {
            status
        } else {
            tt_status::TT_EMPTY
        }
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Searches the scrollback and the screen for `pattern`, literal UTF-8,
/// with `flags` (`TT_SEARCH_IGNORE_CASE` or 0), replacing any earlier
/// search; the frame marks every match in the viewport. An empty pattern
/// ends the search. `TT_INVALID`, ending the search, for text that is not
/// UTF-8, longer than `TT_MAX_SEARCH_PATTERN` bytes, or an unknown flag.
///
/// # Safety
///
/// `term` is `NULL` or live; `pattern` is valid for its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_search_set(
    term: *const tt_term,
    pattern: tt_str,
    flags: u32,
) -> tt_status {
    let body = |t: &tt_term| {
        let term = &mut t.core().lock().term;
        // SAFETY: the caller's contract.
        let pattern = unsafe { text(pattern) };
        match pattern {
            Some("") => term.search_clear(),
            Some(p) if flags & !TT_SEARCH_IGNORE_CASE == 0 => {
                if !term.search_set(p, flags & TT_SEARCH_IGNORE_CASE == 0) {
                    return tt_status::TT_INVALID;
                }
            }
            _ => {
                term.search_clear();
                return tt_status::TT_INVALID;
            }
        }
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Steps to the next match (`forward` 1) or the previous one (0): from the
/// current match, or else from the top (bottom) of the viewport, wrapping
/// round at the ends. The viewport scrolls to show it and the frame marks
/// it `TT_CELL_CURRENT_MATCH`. `TT_OK` with the match written to `*found`
/// (which may be `NULL`), `TT_EMPTY` when there is no search or no match.
///
/// # Safety
///
/// `term` is `NULL` or live; `found` is `NULL` or points to `struct_size`
/// writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_search_step(
    term: *const tt_term,
    forward: u8,
    found: *mut tt_match,
) -> tt_status {
    let body = |t: &tt_term| {
        let m: Option<Match> = {
            let term = &mut t.core().lock().term;
            if forward == 0 {
                term.search_prev(None)
            } else {
                term.search_next(None)
            }
        };
        let Some(m) = m else {
            return tt_status::TT_EMPTY;
        };
        let out = tt_match {
            start_col: m.start.col,
            end_col: m.end.col,
            start_line: m.start.line,
            end_line: m.end.line,
            ..zeroed()
        };
        // SAFETY: the caller's contract; a NULL or too small `found` is
        // skipped, the step having happened.
        if unsafe { can_write(found) } {
            unsafe { write_sized(found, &out) };
        }
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Writes the number of matches, up to `TT_MAX_SEARCH_MATCHES`, to
/// `*count`; 0 without a search. `TT_INVALID` for a `NULL` `count`.
///
/// # Safety
///
/// `term` is `NULL` or live; `count` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_search_count(
    term: *const tt_term,
    count: *mut usize,
) -> tt_status {
    let body = |t: &tt_term| {
        if count.is_null() {
            return tt_status::TT_INVALID;
        }
        let n = t.core().lock().term.search_count();
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { count.write(n) };
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests;
