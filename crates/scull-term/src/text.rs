//! The screen as plain text, for readers that are not renderers: screen
//! readers (NSAccessibility, UI Automation) and copying. Separate from the
//! frame because it reads the grid on demand, in the order and shape a
//! reader wants, while the frame is cut for drawing.

use std::ops::Range;

use scull_grid::{Attrs, CellFlags, Content};

use crate::terminal::Terminal;

/// Read for a cluster id the table no longer has, as the frame draws it.
const MISSING_CLUSTER: &str = "\u{FFFD}";

impl Terminal {
    /// Appends the text of viewport rows `rows` to `out`, one line per row
    /// joined by `\n`, so line N is row N. The halves of wide characters
    /// behind their heads are skipped, blank and hidden (SGR 8) cells read
    /// as spaces, and trailing spaces are trimmed. Rows past the viewport
    /// are not read.
    pub fn read_text(&self, rows: Range<u16>, out: &mut String) {
        let grid = self.grid();
        for r in rows.start..rows.end.min(grid.screen_rows()) {
            if r > rows.start {
                out.push('\n');
            }
            let start = out.len();
            for cell in grid.visible_row(r).into_iter().flat_map(|row| row.cells()) {
                if cell.flags().contains(CellFlags::SPACER) {
                    continue;
                }
                // Concealed text, such as a typed password, is not read out.
                let hidden = grid
                    .styles()
                    .get(cell.style())
                    .is_some_and(|s| s.attrs.contains(Attrs::HIDDEN));
                match cell.content() {
                    Content::Char(c) if !hidden => out.push(c),
                    Content::Cluster(id) if !hidden => {
                        out.push_str(grid.clusters().get(id).unwrap_or(MISSING_CLUSTER));
                    }
                    _ => out.push(' '),
                }
            }
            let kept = out
                .get(start..)
                .map_or(0, |line| line.trim_end_matches(' ').len());
            out.truncate(start + kept);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(feed: &str, rows: Range<u16>) -> String {
        let mut t = Terminal::new(8, 3, 10).unwrap();
        t.feed(feed.as_bytes());
        let mut out = String::new();
        t.read_text(rows, &mut out);
        out
    }

    #[test]
    fn each_row_is_a_line_without_its_trailing_blanks() {
        assert_eq!(text("ab  c\r\n\r\nx", 0..3), "ab  c\n\nx");
        assert_eq!(text("ab\r\ncd", 1..2), "cd");
        assert_eq!(text("ab", 2..99), "", "rows past the viewport are not read");
        assert_eq!(text("ab", Range { start: 2, end: 1 }), "");
    }

    #[test]
    fn wide_characters_and_clusters_read_once() {
        assert_eq!(
            text("\u{4e2d}x\x1b[?2027he\u{301}", 0..1),
            "\u{4e2d}xe\u{301}"
        );
    }

    #[test]
    fn a_wrapped_line_reads_as_two_and_erased_cells_as_spaces() {
        assert_eq!(text("abcdefghij", 0..2), "abcdefgh\nij");
        assert_eq!(text("abc\x1b[2D\x1b[X", 0..1), "a c");
    }

    #[test]
    fn concealed_text_reads_as_blanks() {
        assert_eq!(text("pw: \x1b[8msecret\x1b[m!", 0..2), "pw:\n  !");
    }

    #[test]
    fn the_viewport_follows_the_history() {
        let mut t = Terminal::new(4, 2, 10).unwrap();
        t.feed(b"1\r\n2\r\n3");
        t.scroll_display(1);
        let mut out = String::new();
        t.read_text(0..2, &mut out);
        assert_eq!(out, "1\n2");
    }
}
