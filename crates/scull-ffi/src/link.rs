//! OSC 8 links under the host's pointer: which link a cell holds, so a
//! click can open it, and the one to underline while the open modifier is
//! held. The URI comes from `tt_term_link_uri`; it is program output, and
//! the host decides what it may open.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use scull_term::FrameCell;

use crate::guard::{tt_status, with_term};
use crate::term::tt_term;

/// `tt_cell.flags`: the cell carries the link `tt_term_link_hover` set.
pub const TT_CELL_LINK: u8 = 16;

const _: () = assert!(TT_CELL_LINK == FrameCell::LINK);

/// Writes the link id of viewport cell `row`, `col` to `*id`, for
/// `tt_term_link_uri`. `TT_EMPTY` (and 0) where no link covers the cell
/// or the cell is off the viewport; `TT_INVALID` for a `NULL` `id`.
///
/// # Safety
///
/// `term` is `NULL` or live; `id` is `NULL` or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_link_at(
    term: *const tt_term,
    row: u16,
    col: u16,
    id: *mut u32,
) -> tt_status {
    let body = |t: &tt_term| {
        if id.is_null() {
            return tt_status::TT_INVALID;
        }
        let found = t.core().lock().term.link_at(row, col);
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { id.write(found.unwrap_or(0)) };
        if found.is_some() {
            tt_status::TT_OK
        } else {
            tt_status::TT_EMPTY
        }
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

/// Underlines link `id`: from the next `tt_frame_update` every viewport
/// cell carrying it has `TT_CELL_LINK`, across rows when it wraps. 0
/// clears it; so does an id no live link has, and the core dropping the
/// link later.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_link_hover(term: *const tt_term, id: u32) -> tt_status {
    let body = |t: &tt_term| {
        t.core().lock().term.hover_link((id != 0).then_some(id));
        tt_status::TT_OK
    };
    // SAFETY: the caller's contract.
    unsafe { with_term(term, body) }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;
    use crate::frame::{tt_frame_free, tt_frame_new, tt_frame_update, tt_frame_view};
    use crate::guard::zeroed;
    use crate::term::tests::{feed, new_term};
    use crate::term::tt_term_free;

    fn link_at(term: *const tt_term, row: u16, col: u16) -> (tt_status, u32) {
        let mut id = u32::MAX;
        // SAFETY: a writable id.
        let status = unsafe { tt_term_link_at(term, row, col, &mut id) };
        (status, id)
    }

    fn row_flags(term: *const tt_term, row: usize) -> Vec<u8> {
        let frame = tt_frame_new();
        let mut view: tt_frame_view = zeroed();
        view.struct_size = u32::try_from(size_of::<tt_frame_view>()).unwrap();
        // SAFETY: live handles and a whole view.
        assert_eq!(
            unsafe { tt_frame_update(frame, term, &mut view) },
            tt_status::TT_OK
        );
        let cols = usize::from(view.cols);
        // SAFETY: the view's cells stay valid until the frame is freed.
        let cells = unsafe { std::slice::from_raw_parts(view.cells, view.cells_len) };
        let flags = cells[row * cols..(row + 1) * cols]
            .iter()
            .map(|c| c.flags)
            .collect();
        // SAFETY: the frame made above.
        unsafe { tt_frame_free(frame) };
        flags
    }

    #[test]
    fn a_linked_cell_answers_its_id_and_hovering_marks_it() {
        let term = new_term(6, 2);
        feed(term, b"a\x1b]8;;https://example.com\x1b\\bc\x1b]8;;\x1b\\");
        let (status, id) = link_at(term, 0, 1);
        assert_eq!(status, tt_status::TT_OK);
        assert_ne!(id, 0);
        assert_eq!(link_at(term, 0, 0), (tt_status::TT_EMPTY, 0));
        assert_eq!(link_at(term, 5, 1), (tt_status::TT_EMPTY, 0));
        // SAFETY: live handle.
        assert_eq!(unsafe { tt_term_link_hover(term, id) }, tt_status::TT_OK);
        const L: u8 = TT_CELL_LINK;
        assert_eq!(row_flags(term, 0), [0, L, L, 0, 0, 0]);
        // SAFETY: live handle.
        assert_eq!(unsafe { tt_term_link_hover(term, 0) }, tt_status::TT_OK);
        assert_eq!(row_flags(term, 0), [0; 6]);
        // SAFETY: the handle made above.
        unsafe { tt_term_free(term) };
    }

    #[test]
    fn null_handles_and_ids_are_refused() {
        let term = new_term(4, 1);
        // SAFETY: a live handle and a NULL id, which is refused.
        let status = unsafe { tt_term_link_at(term, 0, 0, ptr::null_mut()) };
        assert_eq!(status, tt_status::TT_INVALID);
        let mut id = 0;
        // SAFETY: NULL handles are refused.
        unsafe {
            assert_eq!(
                tt_term_link_at(ptr::null(), 0, 0, &mut id),
                tt_status::TT_INVALID
            );
            assert_eq!(tt_term_link_hover(ptr::null(), 1), tt_status::TT_INVALID);
            tt_term_free(term);
        }
    }
}
