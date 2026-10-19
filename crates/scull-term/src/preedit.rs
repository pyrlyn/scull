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

/// How an input method treats a stretch of its composing text. Renderers
/// draw them apart so the user can tell what is still raw from what the
/// engine converted, and which clause a candidate list is open on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ClauseKind {
    /// Typed and not converted yet (a kana before its conversion).
    #[default]
    Input = 0,
    /// Converted, not the one being edited.
    Converted = 1,
    /// The clause the user is converting now.
    Target = 2,
}

/// A stretch of the preedit text, as byte offsets, with its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreeditClause {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
    /// How the input method treats the stretch.
    pub kind: ClauseKind,
}

/// A run of columns of the shown preedit with one kind, for a renderer
/// that draws the clauses apart. Spans are in column order and do not
/// overlap; none is listed for a preedit set without clauses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PreeditSpan {
    /// First column.
    pub col: u16,
    /// Columns covered.
    pub cols: u16,
    /// What the run is.
    pub kind: ClauseKind,
}

/// The preedit as the UI set it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Preedit {
    text: String,
    /// Byte offset of the caret in `text`, on a character boundary.
    caret: usize,
    /// In text order, non-empty, on character boundaries, not overlapping.
    clauses: Vec<(Range<usize>, ClauseKind)>,
}

impl Preedit {
    /// Replaces the text, cut at [`MAX_PREEDIT_BYTES`]; the caret is moved
    /// back onto a character boundary inside it.
    pub(crate) fn set(&mut self, text: &str, caret: usize) {
        let text = text.get(..text.floor_char_boundary(MAX_PREEDIT_BYTES));
        self.text.clear();
        self.text.push_str(text.unwrap_or_default());
        self.caret = self.text.floor_char_boundary(caret);
        self.clauses.clear();
    }

    /// Marks stretches of the text set last. The input method is not
    /// trusted to be tidy, so every clause is cut to the text and onto
    /// character boundaries, and one that starts inside the one before it
    /// loses the overlap. Text no clause names is [`ClauseKind::Input`].
    /// At most one clause per byte of text is kept, so the list is bounded.
    pub(crate) fn set_clauses(&mut self, clauses: &[PreeditClause]) {
        self.clauses.clear();
        let mut ordered: Vec<&PreeditClause> = clauses.iter().collect();
        ordered.sort_by_key(|c| c.start);
        let mut from = 0;
        for c in ordered {
            let start = self.text.floor_char_boundary(c.start.min(self.text.len()));
            let start = start.max(from);
            let end = self.text.floor_char_boundary(c.end.min(self.text.len()));
            if start < end {
                self.clauses.push((start..end, c.kind));
                from = end;
            }
        }
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
        let spans = self.spans(&shown, narrow(start));
        Some(PreeditLayout {
            row,
            col: narrow(start),
            cols: narrow(total),
            caret: narrow(caret),
            clusters: shown,
            spans,
        })
    }

    /// The shown clusters grouped by clause kind, from column `col`; a
    /// cluster belongs to the clause its first byte is in.
    fn spans(&self, shown: &[(Range<usize>, u8)], col: u16) -> Vec<PreeditSpan> {
        let mut spans: Vec<PreeditSpan> = Vec::new();
        if self.clauses.is_empty() {
            return spans;
        }
        let mut at = col;
        for (range, width) in shown {
            let kind = self
                .clauses
                .iter()
                .find(|(c, _)| c.contains(&range.start))
                .map_or(ClauseKind::Input, |&(_, k)| k);
            let width = u16::from(*width);
            match spans.last_mut() {
                Some(last) if last.kind == kind => last.cols += width,
                _ => spans.push(PreeditSpan {
                    col: at,
                    cols: width,
                    kind,
                }),
            }
            at += width;
        }
        spans
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
    /// The clusters grouped by clause; empty when none were set.
    pub(crate) spans: Vec<PreeditSpan>,
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

    fn clauses(
        text: &str,
        set: &[(usize, usize, ClauseKind)],
        cursor: (u16, u16),
        cols: u16,
    ) -> Vec<PreeditSpan> {
        let mut p = Preedit::default();
        p.set(text, text.len());
        let list: Vec<_> = set
            .iter()
            .map(|&(start, end, kind)| PreeditClause { start, end, kind })
            .collect();
        p.set_clauses(&list);
        p.layout(
            cursor,
            cols,
            WidthOptions::default(),
            ClusterPolicy::default(),
        )
        .unwrap()
        .spans
    }

    fn span(col: u16, cols: u16, kind: ClauseKind) -> PreeditSpan {
        PreeditSpan { col, cols, kind }
    }

    #[test]
    fn clauses_become_runs_of_columns() {
        use ClauseKind::{Converted, Input, Target};
        // 日本 converted, ご the target, "x" still input.
        let text = "日本ごx";
        let got = clauses(
            text,
            &[(0, 6, Converted), (6, 9, Target), (9, 10, Input)],
            (0, 2),
            20,
        );
        assert_eq!(
            got,
            [span(2, 4, Converted), span(6, 2, Target), span(8, 1, Input)]
        );
    }

    #[test]
    fn text_no_clause_names_is_input_and_neighbours_of_one_kind_merge() {
        use ClauseKind::{Converted, Input};
        let got = clauses("abcd", &[(0, 1, Converted), (1, 2, Converted)], (0, 0), 10);
        assert_eq!(got, [span(0, 2, Converted), span(2, 2, Input)]);
    }

    #[test]
    fn no_clauses_no_spans() {
        assert_eq!(clauses("abc", &[], (0, 0), 10), []);
    }

    #[test]
    fn untidy_clauses_are_cut_to_the_text_and_never_overlap() {
        use ClauseKind::{Converted, Target};
        // Out of order, past the end, inside a character, overlapping, empty.
        let got = clauses(
            "日本",
            &[
                (3, 99, Target),
                (0, 4, Converted),
                (5, 5, Target),
                (2, 1, Target),
            ],
            (0, 0),
            10,
        );
        assert_eq!(got, [span(0, 2, Converted), span(2, 2, Target)]);
    }

    #[test]
    fn spans_follow_the_clusters_that_stay_in_view() {
        use ClauseKind::{Converted, Target};
        let got = clauses("abcdefgh", &[(0, 4, Converted), (4, 8, Target)], (0, 3), 4);
        assert_eq!(got, [span(0, 4, Target)], "the left clause scrolled out");
    }

    #[test]
    fn setting_the_text_again_drops_the_clauses() {
        let mut p = Preedit::default();
        p.set("abc", 3);
        p.set_clauses(&[PreeditClause {
            start: 0,
            end: 3,
            kind: ClauseKind::Target,
        }]);
        p.set("abc", 3);
        let l = p.layout(
            (0, 0),
            10,
            WidthOptions::default(),
            ClusterPolicy::default(),
        );
        assert!(l.unwrap().spans.is_empty());
    }
}
