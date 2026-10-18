//! Jumping between the shell's prompts. A shell that speaks OSC 133 marks
//! where each prompt starts; the core keeps those lines in the scrollback
//! and moves the viewport on request. Kept apart from the other scroll
//! calls so a host can feature-test it by its own entry point.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use crate::guard::{tt_status, with_term};
use crate::term::tt_term;

/// Scrolls the viewport so the previous (`forward` 0) or next (`forward`
/// non-zero) prompt the shell marked with OSC 133 `A` is its top row, or
/// the live screen when that prompt is on it. `TT_EMPTY`, and nothing
/// moves, when there is no such prompt: none marked, none above the
/// viewport, or the next one already in view at the bottom. Marks follow
/// the text through a resize, and go with the lines the scrollback's
/// trimming, ED 2, ED 3 or a full reset remove. Call `tt_frame_update`
/// afterwards.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_scroll_to_prompt(term: *const tt_term, forward: u8) -> tt_status {
    let body = |t: &tt_term| {
        if t.core().lock().term.scroll_to_prompt(forward != 0) {
            tt_status::TT_OK
        } else {
            tt_status::TT_EMPTY
        }
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

    fn offset(term: *const tt_term) -> usize {
        // SAFETY: live handle.
        unsafe { &*term }.core().lock().term.grid().display_offset()
    }

    #[test]
    fn the_viewport_jumps_between_marked_prompts() {
        let term = new_term(10, 3);
        for i in 0..6 {
            feed(term, format!("\x1b]133;A\x07$ {i}\r\n").as_bytes());
        }
        // SAFETY: live handle.
        unsafe {
            assert_eq!(tt_term_scroll_to_prompt(term, 0), tt_status::TT_OK);
            assert_eq!(offset(term), 1);
            assert_eq!(tt_term_scroll_to_prompt(term, 0), tt_status::TT_OK);
            assert_eq!(offset(term), 2);
            assert_eq!(tt_term_scroll_to_prompt(term, 1), tt_status::TT_OK);
            assert_eq!(offset(term), 1);
            assert_eq!(tt_term_scroll_to_prompt(term, 1), tt_status::TT_OK);
            assert_eq!(offset(term), 0);
            assert_eq!(tt_term_scroll_to_prompt(term, 1), tt_status::TT_EMPTY);
            tt_term_free(term);
        }
    }

    #[test]
    fn no_marks_answer_empty_and_a_bad_handle_invalid() {
        let term = new_term(10, 3);
        feed(term, b"1\r\n2\r\n3\r\n4\r\n");
        // SAFETY: live handle, and a null one the call refuses.
        unsafe {
            assert_eq!(tt_term_scroll_to_prompt(term, 0), tt_status::TT_EMPTY);
            assert_eq!(tt_term_scroll_to_prompt(term, 1), tt_status::TT_EMPTY);
            assert_eq!(
                tt_term_scroll_to_prompt(ptr::null(), 0),
                tt_status::TT_INVALID
            );
            let _ = with_term(term, |_| panic!("core bug"));
            assert_eq!(tt_term_scroll_to_prompt(term, 0), tt_status::TT_POISONED);
            tt_term_free(term);
        }
    }
}
