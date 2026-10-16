//! The input method's composing text (preedit), laid over the frame at the
//! cursor. It lives in the frame, not the terminal: it belongs to the view
//! the user types into, never reaches the child, and setting it takes no
//! lock. Laying it out in the core gives it the grid's clusters and widths,
//! so every renderer draws it like any other row text with no code of its
//! own (`docs/research/ffi-native-ui.md` §4.5).

use std::ops::Range;

use scull_unicode::{ClusterPolicy, GraphemeState, WidthOptions, cluster_width};

/// Most bytes of preedit a frame keeps. The text comes from an input
/// method, not the child, but it is still capped: a composition is a few
/// words, and the layout costs a pass over every byte per update.
pub const MAX_PREEDIT_BYTES: usize = 1024;

/// The preedit as the UI set it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Preedit {
    text: String,
    /// Byte offset of the caret in `text`, on a character boundary.
    caret: usize,
}

impl Preedit {
    /// Replaces the text, cut at [`MAX_PREEDIT_BYTES`]; the caret is moved
    /// back onto a character boundary inside it.
    pub(crate) fn set(&mut self, text: &str, caret: usize) {
        let text = text.get(..text.floor_char_boundary(MAX_PREEDIT_BYTES));
        self.text.clear();
        self.text.push_str(text.unwrap_or_default());
        self.caret = self.text.floor_char_boundary(caret);
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Lays the preedit out at the cursor (`row`, `col`) of a row `cols`
    /// wide. `None` when there is nothing to show.
    pub(crate) fn layout(
        &self,
        (row, col): (u16, u16),
        cols: u16,
        width: WidthOptions,
        policy: ClusterPolicy,
    ) -> Option<PreeditLayout> {
        if cols == 0 {
            return None;
        }
        let mut clusters: Vec<(Range<usize>, u8)> = Vec::new();
        let mut segments = GraphemeState::new(width.version);
        for (at, ch) in self.text.char_indices() {
            let end = at + ch.len_utf8();
            let starts = segments.next(ch);
            match clusters.last_mut() {
                Some((range, _)) if !starts => range.end = end,
                _ => clusters.push((at..end, 0)),
            }
        }
        for (range, cells) in &mut clusters {
            let text = self.text.get(range.clone()).unwrap_or_default();
            // A lone mark still takes a cell, or it would vanish.
            *cells = cluster_width(text, width, policy).clamp(1, 2);
        }
        let caret: u32 = clusters
            .iter()
            .filter(|(r, _)| r.end <= self.caret)
            .map(|&(_, w)| u32::from(w))
            .sum();
        let cols = u32::from(cols);
        let mut total: u32 = clusters.iter().map(|&(_, w)| u32::from(w)).sum();
        // Too wide for the row: keep the caret in view, dropping from the
        // left first, then cut the right.
        let mut dropped = 0;
        let mut first = 0;
        while total > cols && caret.saturating_sub(dropped) >= cols {
            let w = clusters.get(first).map_or(0, |&(_, w)| u32::from(w));
            (total, dropped, first) = (total - w, dropped + w, first + 1);
        }
        let mut shown = clusters.get(first..).unwrap_or_default().to_vec();
        while total > cols {
            total -= shown.pop().map_or(0, |(_, w)| u32::from(w));
        }
        if total == 0 {
            return None;
        }
        // Shifted left so it ends inside the row, as a line editor would wrap.
        let start = u32::from(col).min(cols - total);
        let caret = (start + caret.saturating_sub(dropped)).min(cols - 1);
        let narrow = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
        Some(PreeditLayout {
            row,
            col: narrow(start),
            cols: narrow(total),
            caret: narrow(caret),
            clusters: shown,
        })
    }
}

/// The preedit placed on one row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct PreeditLayout {
    pub(crate) row: u16,
    /// First column.
    pub(crate) col: u16,
    /// Columns covered.
    pub(crate) cols: u16,
    /// Column of the caret: where the frame's cursor goes.
    pub(crate) caret: u16,
    /// The clusters shown, as byte ranges of the preedit, with their widths.
    pub(crate) clusters: Vec<(Range<usize>, u8)>,
}

impl PreeditLayout {
    pub(crate) fn covers(&self, col: u16) -> bool {
        (self.col..self.col.saturating_add(self.cols)).contains(&col)
    }
}

/// Where the frame shows the preedit, for a renderer that marks it (an
/// underline). The text itself is already in the row's cells and runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePreedit {
    /// Viewport row.
    pub row: u16,
    /// First column.
    pub col: u16,
    /// Columns covered.
    pub cols: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(text: &str, caret: usize, cursor: (u16, u16), cols: u16) -> Option<PreeditLayout> {
        let mut p = Preedit::default();
        p.set(text, caret);
        p.layout(
            cursor,
            cols,
            WidthOptions::default(),
            ClusterPolicy::default(),
        )
    }

    fn shown<'a>(text: &'a str, l: &PreeditLayout) -> Vec<&'a str> {
        l.clusters.iter().map(|(r, _)| &text[r.clone()]).collect()
    }

    #[test]
    fn cjk_takes_two_cells_each_and_the_caret_follows_the_text() {
        let l = layout("日本ご", 6, (1, 2), 20).unwrap();
        assert_eq!((l.row, l.col, l.cols, l.caret), (1, 2, 6, 6));
        assert_eq!(
            l.clusters.iter().map(|c| c.1).collect::<Vec<_>>(),
            [2, 2, 2]
        );
    }

    #[test]
    fn a_cluster_is_one_unit() {
        let text = "e\u{301}x";
        let l = layout(text, 1, (0, 0), 10).unwrap();
        assert_eq!(shown(text, &l), ["e\u{301}", "x"]);
        assert_eq!(
            (l.cols, l.caret),
            (2, 0),
            "a caret inside a cluster stays before it"
        );
    }

    #[test]
    fn near_the_right_edge_it_shifts_left_to_fit() {
        let l = layout("abcd", 4, (0, 8), 10).unwrap();
        assert_eq!((l.col, l.cols, l.caret), (6, 4, 9));
    }

    #[test]
    fn wider_than_the_row_it_keeps_the_caret_in_view() {
        let text = "abcdefghij";
        let l = layout(text, 10, (0, 3), 4).unwrap();
        assert_eq!(shown(text, &l), ["g", "h", "i", "j"]);
        assert_eq!((l.col, l.caret), (0, 3));
        let l = layout(text, 0, (0, 3), 4).unwrap();
        assert_eq!(shown(text, &l), ["a", "b", "c", "d"]);
        assert_eq!(l.caret, 0);
    }

    #[test]
    fn a_wide_cluster_that_cannot_fit_is_not_shown() {
        assert_eq!(layout("日", 0, (0, 0), 1), None);
        assert_eq!(layout("", 0, (0, 0), 10), None);
    }

    #[test]
    fn text_past_the_cap_is_cut_on_a_character_boundary() {
        let mut p = Preedit::default();
        let long = "日".repeat(MAX_PREEDIT_BYTES);
        p.set(&long, usize::MAX);
        assert!(p.text().len() <= MAX_PREEDIT_BYTES);
        assert_eq!(p.text().len() % 3, 0);
        assert_eq!(p.caret, p.text().len());
        p.set("日本", 1);
        assert_eq!(p.caret, 0, "a caret inside a character moves back");
    }
}
