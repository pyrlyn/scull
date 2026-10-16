//! The OSC 8 link under the host's pointer: which link a viewport cell
//! holds, and the one hovered while the open modifier is down. The hovered
//! link is marked in the frame like a selection, so a host underlines it
//! without knowing its extent: a wrapped URL spans rows.

use scull_grid::{LinkId, Row};

use crate::frame::{FrameCell, Mark};
use crate::terminal::Terminal;

impl Terminal {
    /// The link id of viewport cell `row`, `col`, for [`Terminal::link_uri`];
    /// `None` off the viewport or where no link covers the cell.
    pub fn link_at(&self, row: u16, col: u16) -> Option<u32> {
        if col > self.last_col() {
            return None;
        }
        self.viewport_row_of(row)?.link_at(col).map(|id| id.0)
    }

    /// Marks link `id` hovered, so its cells carry [`FrameCell::LINK`] from
    /// the next frame on; `None`, or an id no live link has, clears it.
    pub fn hover_link(&mut self, id: Option<u32>) {
        let live = id.filter(|&id| self.state.links.uri(id).is_some());
        self.state.links.hovered = live.map(LinkId);
    }

    /// The hovered link's id.
    pub fn hovered_link(&self) -> Option<u32> {
        self.state.links.hovered.map(|id| id.0)
    }

    /// Lays the hovered link over the viewport cells that carry it.
    pub(crate) fn link_marks(&self, out: &mut Vec<Mark>) {
        let Some(id) = self.state.links.hovered else {
            return;
        };
        for row in 0..self.grid().screen_rows() {
            let Some(line) = self.viewport_row_of(row) else {
                continue;
            };
            for span in line.links().iter().filter(|s| s.link == id) {
                out.push(Mark {
                    row,
                    start: span.start,
                    end: span.end,
                    flags: FrameCell::LINK,
                });
            }
        }
    }

    // `viewport_point` clamps, which would answer for the bottom row when
    // the host asks below it.
    fn viewport_row_of(&self, row: u16) -> Option<&Row> {
        if row >= self.grid().screen_rows() {
            return None;
        }
        self.line_row(self.viewport_point(row, 0).line)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::frame::{Frame, FrameCell};
    use crate::terminal::Terminal;

    const L: u8 = FrameCell::LINK;

    fn link(uri: &str, text: &str) -> Vec<u8> {
        format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\").into_bytes()
    }

    fn flags(f: &Frame, r: u16) -> Vec<u8> {
        f.row_cells(r).iter().map(|c| c.flags).collect()
    }

    fn update(t: &mut Terminal, f: &mut Frame) {
        assert!(t.update_frame(f, Instant::now()));
    }

    #[test]
    fn a_cell_answers_the_link_it_carries() {
        let mut t = Terminal::new(8, 3, 0).unwrap();
        t.feed(b"a");
        t.feed(&link("https://example.com", "bc"));
        let id = t.link_at(0, 1).unwrap();
        assert_eq!(t.link_at(0, 2), Some(id));
        assert_eq!(t.link_uri(id), Some("https://example.com"));
        assert_eq!(t.link_at(0, 0), None);
        assert_eq!(t.link_at(0, 3), None);
        assert_eq!(t.link_at(1, 1), None);
        assert_eq!(t.link_at(3, 1), None, "below the viewport");
        assert_eq!(t.link_at(0, 8), None, "right of it");
    }

    #[test]
    fn the_hovered_link_is_marked_across_its_wrap_and_nowhere_else() {
        let mut t = Terminal::new(8, 3, 0).unwrap();
        t.feed(b"xxxxxx");
        t.feed(&link("https://a.example", "abcd"));
        // The same URI with no id is another link: it stays plain.
        t.feed(&link("https://a.example", "z"));
        let mut f = Frame::new();
        update(&mut t, &mut f);
        let id = t.link_at(1, 0).unwrap();
        assert_eq!(t.link_at(0, 6), Some(id));
        t.hover_link(Some(id));
        assert_eq!(t.hovered_link(), Some(id));
        update(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [0, 1]);
        assert_eq!(flags(&f, 0), [0, 0, 0, 0, 0, 0, L, L]);
        assert_eq!(flags(&f, 1), [L, L, 0, 0, 0, 0, 0, 0]);
        t.hover_link(None);
        update(&mut t, &mut f);
        assert_eq!(f.damage().dirty(), [0, 1]);
        assert_eq!(flags(&f, 0), [0; 8]);
        assert_eq!(flags(&f, 1), [0; 8]);
    }

    #[test]
    fn a_link_scrolled_into_history_is_found_where_the_viewport_shows_it() {
        let mut t = Terminal::new(8, 2, 10).unwrap();
        t.feed(&link("mailto:a@example.com", "m"));
        t.feed(b"\r\n1\r\n2\r\n3");
        assert_eq!(t.link_at(0, 0), None);
        t.scroll_display(3);
        let id = t.link_at(0, 0).unwrap();
        assert_eq!(t.link_uri(id), Some("mailto:a@example.com"));
        t.hover_link(Some(id));
        let mut f = Frame::new();
        update(&mut t, &mut f);
        assert_eq!(flags(&f, 0)[0], L);
    }

    #[test]
    fn an_unknown_id_hovers_nothing() {
        let mut t = Terminal::new(8, 2, 0).unwrap();
        t.feed(&link("https://example.com", "a"));
        t.hover_link(Some(99));
        assert_eq!(t.hovered_link(), None);
        t.hover_link(Some(0));
        assert_eq!(t.hovered_link(), None);
    }
}
