//! Resizing the terminal: both screens take the new size, the cursor and
//! the saved cursors follow their cells, and what is sized to the screen
//! (margins, tab stops) is rebuilt. Separate from `screen.rs` because it
//! drives the grid's reflow instead of switching screens.
//!
//! The primary screen and its scrollback are rewrapped; the alternate
//! screen is cut or padded, because its application redraws on `SIGWINCH`
//! anyway (kitty and WezTerm do the same, T11). The grid moves every
//! tracked point with its cell (`Grid::resize`); this module only says
//! which points those are and brings them back onto the screen. Image
//! placements are tracked by their top-left cell (T11's `TrackPoint`s), so
//! an image stays with the line it was drawn on.

use scull_grid::{Grid, GridError, Reflow, TrackPoint};

use crate::error::TermError;
use crate::images::{self, ScreenImages};
use crate::state::{Cursor, Margins, State};

impl State {
    /// Resizes both screens to `cols` x `rows`. Zero is refused and leaves
    /// everything as it was; the same size is a no-op.
    pub(crate) fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TermError> {
        if cols == 0 || rows == 0 {
            return Err(GridError::Empty.into());
        }
        if (cols, rows) == (self.cols(), self.rows()) {
            return Ok(());
        }
        let alt_active = self.alt_active;
        let same_width = cols == self.cols();
        let (main, alt) = if alt_active {
            (&mut self.alt, &mut self.grid)
        } else {
            (&mut self.grid, &mut self.alt)
        };
        let (main_images, alt_images) = if alt_active {
            (&mut self.alt_images, &mut self.images)
        } else {
            (&mut self.images, &mut self.alt_images)
        };
        let [main_saved, alt_saved] = &mut self.saved;
        // A hidden screen has no live cursor. The main screen's is what
        // DECSC kept on the way to the alternate screen (`?1049`); without
        // one its bottom row stands in, so shrinking pushes its top into
        // history instead of cutting rows off below.
        let hidden_main = Cursor {
            row: main.screen_rows().saturating_sub(1),
            ..Cursor::default()
        };
        let mut main_cursor = match (alt_active, &main_saved) {
            (false, _) => self.cursor,
            (true, Some(saved)) => saved.cursor,
            (true, None) => hidden_main,
        };
        let mut alt_cursor = match (alt_active, &alt_saved) {
            (true, _) => self.cursor,
            (false, Some(saved)) => saved.cursor,
            (false, None) => Cursor::default(),
        };
        let main_saved = main_saved.as_mut().map(|s| &mut s.cursor);
        let alt_saved = alt_saved.as_mut().map(|s| &mut s.cursor);
        resize_screen(
            (main, main_images),
            (cols, rows),
            Reflow::Rewrap,
            &mut main_cursor,
            main_saved,
        )?;
        resize_screen(
            (alt, alt_images),
            (cols, rows),
            Reflow::Truncate,
            &mut alt_cursor,
            alt_saved,
        )?;
        self.cursor = if alt_active { alt_cursor } else { main_cursor };
        // A wrap pending at the old right edge means nothing at another width.
        self.cursor.pending_wrap &= same_width;
        self.margins = Margins::full(cols, rows);
        self.tabs.resize(cols);
        self.end_cluster();
        Ok(())
    }
}

/// Resizes `grid` and moves `cursor`, `saved` (screen positions) and the
/// image placements with their cells, then clamps both cursors onto the
/// screen.
fn resize_screen(
    (grid, images): (&mut Grid, &mut ScreenImages),
    (cols, rows): (u16, u16),
    reflow: Reflow,
    cursor: &mut Cursor,
    saved: Option<&mut Cursor>,
) -> Result<(), GridError> {
    let point = |c: &Cursor, base: usize| TrackPoint::new(base + usize::from(c.row), c.col);
    let base = grid.history_len();
    let mut at = point(cursor, base);
    let mut points: Vec<TrackPoint> = saved.iter().map(|s| point(s, base)).collect();
    images.prune();
    let first_anchor = points.len();
    points.extend(images::anchors(images));
    grid.resize(cols, rows, reflow, &mut at, &mut points)?;
    images::reanchor(images, points.get(first_anchor..).unwrap_or_default());
    let base = grid.history_len();
    let onto_screen = |p: TrackPoint, c: &mut Cursor| {
        let row = p.row.saturating_sub(base).min(usize::from(rows - 1));
        c.row = u16::try_from(row).unwrap_or(rows - 1);
        c.col = p.col.min(cols - 1);
    };
    onto_screen(at, cursor);
    if let (Some(s), Some(&p)) = (saved, points.first()) {
        onto_screen(p, s);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use scull_grid::Content;

    use crate::terminal::Terminal;
    use crate::terminal::tests::stream;

    /// The text of screen row `r`, blanks as spaces, trailing ones cut.
    fn line(term: &Terminal, r: u16) -> String {
        let row = term.grid().screen_row(r).unwrap();
        let text: String = row
            .cells()
            .map(|c| match c.content() {
                Content::Char(ch) => ch,
                _ => ' ',
            })
            .collect();
        text.trim_end().to_owned()
    }

    fn at(term: &Terminal) -> (u16, u16) {
        (term.cursor().row, term.cursor().col)
    }

    #[test]
    fn narrowing_rewraps_the_primary_screen_and_moves_the_cursor_with_it() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"abcdefgh");
        term.resize(5, 3).unwrap();
        assert_eq!(
            (line(&term, 0), line(&term, 1)),
            ("abcde".into(), "fgh".into())
        );
        assert_eq!(at(&term), (1, 3));
        term.feed(b"X");
        assert_eq!(line(&term, 1), "fghX");
    }

    #[test]
    fn the_alternate_screen_is_cut_not_rewrapped() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"\x1b[?1049habcdefgh");
        term.resize(5, 3).unwrap();
        assert_eq!(
            (line(&term, 0), line(&term, 1)),
            ("abcde".into(), String::new())
        );
        assert_eq!(at(&term), (0, 4));
    }

    #[test]
    fn the_hidden_main_screen_is_rewrapped_and_its_saved_cursor_follows() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"abcdefgh\x1b[?1049hxyz");
        term.resize(5, 3).unwrap();
        term.feed(b"\x1b[?1049l");
        assert_eq!(
            (line(&term, 0), line(&term, 1)),
            ("abcde".into(), "fgh".into())
        );
        assert_eq!(at(&term), (1, 3));
    }

    #[test]
    fn a_saved_cursor_follows_its_cell() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"abcdefg\x1b7\r\n");
        term.resize(5, 3).unwrap();
        term.feed(b"\x1b8");
        assert_eq!(at(&term), (1, 2));
    }

    #[test]
    fn a_pending_wrap_survives_a_height_change_only() {
        let mut term = Terminal::new(4, 3, 10).unwrap();
        term.feed(b"abcd");
        assert!(term.cursor().pending_wrap);
        term.resize(4, 5).unwrap();
        assert!(term.cursor().pending_wrap);
        term.resize(6, 5).unwrap();
        assert!(!term.cursor().pending_wrap);
    }

    #[test]
    fn margins_reset_and_tab_stops_are_kept_and_extended() {
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(b"\x1b[2;4r\x1b[1;4H\x1bH\x1b[H");
        term.resize(20, 6).unwrap();
        assert_eq!(term.margins(), crate::Margins::full(20, 6));
        term.feed(b"\t");
        assert_eq!(at(&term), (0, 3), "the custom stop survives");
        term.feed(b"\t\t");
        assert_eq!(at(&term), (0, 16), "new columns get power-on stops");
    }

    #[test]
    fn zero_is_refused_and_changes_nothing() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"abc");
        assert!(term.resize(0, 3).is_err());
        assert!(term.resize(10, 0).is_err());
        assert_eq!((term.grid().cols(), term.grid().screen_rows()), (10, 3));
        assert_eq!(at(&term), (0, 3));
    }

    proptest! {
        #[test]
        fn no_stream_and_resize_leave_the_cursor_or_margins_off_screen(
            before in stream(),
            after in stream(),
            cols in 1u16..24,
            rows in 1u16..8,
        ) {
            let mut term = Terminal::new(10, 4, 8).unwrap();
            term.feed(&before);
            term.resize(cols, rows).unwrap();
            let c = term.cursor();
            prop_assert!(c.row < rows && c.col < cols);
            term.feed(&after);
            let c = term.cursor();
            prop_assert!(c.row < rows && c.col < cols);
            let m = term.margins();
            prop_assert!(m.bottom < rows && m.right < cols);
        }
    }
}
