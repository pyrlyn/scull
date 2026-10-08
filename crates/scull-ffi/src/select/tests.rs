use std::ptr;

use super::*;
use crate::term::tests::{feed, new_term};
use crate::term::tt_term_free;

fn copied(term: *const tt_term, cap: usize) -> (tt_status, usize, Vec<u8>) {
    let mut buf = vec![0xAA; cap];
    let mut len = usize::MAX;
    // SAFETY: a buffer of `cap` bytes and a writable length.
    let status = unsafe { tt_term_selection_text(term, buf.as_mut_ptr(), cap, &mut len) };
    (status, len, buf)
}

fn pattern(text: &str) -> tt_str {
    tt_str {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

fn search(term: *const tt_term, text: &str, flags: u32) -> tt_status {
    // SAFETY: a valid string.
    unsafe { tt_term_search_set(term, pattern(text), flags) }
}

fn count(term: *const tt_term) -> usize {
    let mut n = usize::MAX;
    // SAFETY: a writable count.
    assert_eq!(
        unsafe { tt_term_search_count(term, &mut n) },
        tt_status::TT_OK
    );
    n
}

fn step(term: *const tt_term, forward: bool) -> (tt_status, tt_match) {
    let mut m = tt_match {
        struct_size: u32::try_from(size_of::<tt_match>()).unwrap(),
        ..zeroed()
    };
    // SAFETY: a whole, writable match.
    let status = unsafe { tt_term_search_step(term, u8::from(forward), &mut m) };
    (status, m)
}

#[test]
fn a_selection_is_made_extended_copied_and_cleared() {
    let term = new_term(8, 3);
    feed(term, "ab cd\u{4e2d}\r\nxyz".as_bytes());
    assert_eq!(copied(term, 8).0, tt_status::TT_EMPTY, "nothing selected");
    // SAFETY: a live handle.
    unsafe {
        assert_eq!(
            tt_term_select_start(term, TT_SELECT_WORD, 0, 3),
            tt_status::TT_OK
        );
    }
    let text = "cd\u{4e2d}";
    let (status, len, buf) = copied(term, 16);
    assert_eq!((status, len), (tt_status::TT_OK, text.len()));
    assert_eq!(&buf[..len], text.as_bytes());
    let (status, len, buf) = copied(term, text.len() - 1);
    assert_eq!((status, len), (tt_status::TT_FULL, text.len()));
    assert!(
        buf.iter().all(|&b| b == 0xAA),
        "a short buffer is left alone"
    );
    // SAFETY: a live handle.
    unsafe {
        assert_eq!(
            tt_term_select_start(term, TT_SELECT_CELL, 0, 0),
            tt_status::TT_OK
        );
        assert_eq!(tt_term_select_extend(term, 1, 1), tt_status::TT_OK);
    }
    let (_, len, buf) = copied(term, 32);
    assert_eq!(&buf[..len], "ab cd\u{4e2d}\nxy".as_bytes());
    // SAFETY: a live handle.
    unsafe {
        assert_eq!(
            tt_term_select_start(term, TT_SELECT_BLOCK, 0, 1),
            tt_status::TT_OK
        );
        assert_eq!(tt_term_select_extend(term, 1, 2), tt_status::TT_OK);
    }
    let (_, len, buf) = copied(term, 32);
    assert_eq!(&buf[..len], b"b\nyz", "each row trimmed");
    // SAFETY: a live handle.
    unsafe {
        assert_eq!(
            tt_term_select_start(term, TT_SELECT_LINE, 1, 0),
            tt_status::TT_OK
        );
    }
    let (_, len, buf) = copied(term, 32);
    assert_eq!(&buf[..len], b"xyz");
    // SAFETY: a live handle.
    unsafe {
        assert_eq!(tt_term_select_clear(term), tt_status::TT_OK);
    }
    assert_eq!(copied(term, 8), (tt_status::TT_EMPTY, 0, vec![0xAA; 8]));
    // SAFETY: live, not used again.
    unsafe { tt_term_free(term) };
}

#[test]
fn output_into_the_selection_drops_it() {
    let term = new_term(6, 3);
    feed(term, b"one");
    // SAFETY: a live handle.
    unsafe {
        tt_term_select_start(term, TT_SELECT_LINE, 0, 0);
    }
    assert_eq!(copied(term, 8).0, tt_status::TT_OK);
    feed(term, b"\x1b[1;1HX");
    assert_eq!(copied(term, 8).0, tt_status::TT_EMPTY);
    // SAFETY: live, not used again.
    unsafe { tt_term_free(term) };
}

#[test]
fn a_search_counts_and_steps_through_its_matches() {
    let term = new_term(8, 3);
    feed(term, b"Ab ab\r\nxAB");
    assert_eq!(count(term), 0, "no search yet");
    assert_eq!(step(term, true).0, tt_status::TT_EMPTY);
    assert_eq!(search(term, "ab", 0), tt_status::TT_OK);
    assert_eq!(count(term), 1);
    assert_eq!(search(term, "ab", TT_SEARCH_IGNORE_CASE), tt_status::TT_OK);
    assert_eq!(count(term), 3);
    let (status, m) = step(term, true);
    assert_eq!(status, tt_status::TT_OK);
    assert_eq!(
        (m.start_line, m.start_col, m.end_line, m.end_col),
        (0, 0, 0, 1)
    );
    assert_eq!(step(term, true).1.start_col, 3);
    let (_, m) = step(term, false);
    assert_eq!((m.start_line, m.start_col), (0, 0));
    let (_, m) = step(term, false);
    assert_eq!((m.start_line, m.start_col), (1, 1), "wraps round");
    // SAFETY: a NULL match is skipped, the step still taken.
    let status = unsafe { tt_term_search_step(term, 1, ptr::null_mut()) };
    assert_eq!(status, tt_status::TT_OK);
    assert_eq!(
        search(term, "", 0),
        tt_status::TT_OK,
        "an empty pattern ends it"
    );
    assert_eq!(count(term), 0);
    // SAFETY: live, not used again.
    unsafe { tt_term_free(term) };
}

#[test]
fn bad_patterns_flags_and_buffers_are_refused() {
    let term = new_term(8, 3);
    feed(term, b"abc");
    assert_eq!(search(term, "b", 0), tt_status::TT_OK);
    assert_eq!(search(term, "b", 2), tt_status::TT_INVALID, "unknown flag");
    assert_eq!(count(term), 0, "a refused search ends the old one");
    let long = "x".repeat(TT_MAX_SEARCH_PATTERN + 1);
    assert_eq!(search(term, &long, 0), tt_status::TT_INVALID);
    let bad = [0xFF_u8];
    let not_utf8 = tt_str {
        ptr: bad.as_ptr(),
        len: 1,
    };
    let null = tt_str {
        ptr: ptr::null(),
        len: 3,
    };
    let mut len = 0;
    // SAFETY: each bad argument is refused before any access.
    unsafe {
        assert_eq!(tt_term_search_set(term, not_utf8, 0), tt_status::TT_INVALID);
        assert_eq!(tt_term_search_set(term, null, 0), tt_status::TT_INVALID);
        assert_eq!(
            tt_term_search_count(term, ptr::null_mut()),
            tt_status::TT_INVALID
        );
        assert_eq!(tt_term_select_start(term, 9, 0, 0), tt_status::TT_INVALID);
        assert_eq!(
            tt_term_selection_text(term, ptr::null_mut(), 0, ptr::null_mut()),
            tt_status::TT_INVALID
        );
        assert_eq!(
            tt_term_selection_text(term, ptr::null_mut(), 4, &mut len),
            tt_status::TT_INVALID
        );
        assert_eq!(tt_term_select_clear(ptr::null()), tt_status::TT_INVALID);
        assert_eq!(
            tt_term_select_extend(ptr::null(), 0, 0),
            tt_status::TT_INVALID
        );
        assert_eq!(
            tt_term_search_step(ptr::null(), 1, ptr::null_mut()),
            tt_status::TT_INVALID
        );
        (*term).poison();
        assert_eq!(tt_term_select_clear(term), tt_status::TT_POISONED);
        assert_eq!(
            tt_term_search_set(term, pattern("a"), 0),
            tt_status::TT_POISONED
        );
        tt_term_free(term);
    }
}

#[test]
fn the_frame_flags_selected_and_matched_cells() {
    use crate::frame::{tt_frame_free, tt_frame_new, tt_frame_update, tt_frame_view};
    use crate::{TT_CELL_CURRENT_MATCH, TT_CELL_MATCH, TT_CELL_SELECTED};

    let (frame, term) = (tt_frame_new(), new_term(4, 2));
    feed(term, b"abab");
    let flags = || {
        let mut view: tt_frame_view = zeroed();
        view.struct_size = u32::try_from(size_of::<tt_frame_view>()).unwrap();
        // SAFETY: live handles and a whole view.
        assert_eq!(
            unsafe { tt_frame_update(frame, term, &mut view) },
            tt_status::TT_OK
        );
        // SAFETY: the view's cells are valid until the next update.
        let cells = unsafe { std::slice::from_raw_parts(view.cells, view.cells_len) };
        cells.iter().take(4).map(|c| c.flags).collect::<Vec<_>>()
    };
    // SAFETY: a live handle.
    unsafe { tt_term_select_start(term, TT_SELECT_CELL, 0, 1) };
    assert_eq!(flags(), [0, TT_CELL_SELECTED, 0, 0]);
    // SAFETY: a live handle.
    unsafe { tt_term_select_clear(term) };
    assert_eq!(search(term, "ab", 0), tt_status::TT_OK);
    step(term, true);
    let current = TT_CELL_MATCH | TT_CELL_CURRENT_MATCH;
    assert_eq!(flags(), [current, current, TT_CELL_MATCH, TT_CELL_MATCH]);
    // SAFETY: live handles, not used again.
    unsafe {
        tt_frame_free(frame);
        tt_term_free(term);
    }
}
