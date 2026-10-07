//! Damage between two pictures of the viewport: which rows moved unchanged
//! (scroll damage the UI can blit) and which must be repainted. Separate
//! from the frame so the comparison is tested on stamps alone.
//!
//! No dirty bits are kept: every row already carries a stable id and a
//! generation bumped by each visible change (`scull-grid`'s `Row`), so a
//! picture that kept the `(id, generation)` of each row finds its damage by
//! comparing stamps. A row whose id shows up at another position with the
//! same generation moved without change; runs of such rows with one offset
//! are reported as one scroll, as foot coalesces its scroll damage
//! (`term_damage_scroll`, `docs/research/foot-contour.md` §1.11). Unlike
//! Alacritty, which degrades to full damage while the viewport is scrolled
//! back (`docs/research/wezterm-alacritty.md`, `term/mod.rs:482-483`), ids
//! make viewport scrolling and region scrolling the same case.

use std::ops::Range;

use scull_grid::{Row, RowId};

/// What a picture remembers of one row to tell whether it changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowStamp {
    /// The logical line shown.
    pub id: RowId,
    /// Its generation when the picture was taken.
    pub generation: u32,
}

impl RowStamp {
    /// The stamp of `row` as it is now.
    pub fn of(row: &Row) -> Self {
        Self {
            id: row.id(),
            generation: row.generation(),
        }
    }
}

/// Rows `start..end` of the new picture are, unchanged, the rows from
/// `from` of the previous one. All scrolls of one update read the previous
/// picture, so a UI blits them from its last frame, in any order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    /// First row moved into.
    pub start: u16,
    /// One past the last row moved into.
    pub end: u16,
    /// Where `start` was in the previous picture.
    pub from: u16,
}

impl Scroll {
    /// The rows moved into.
    pub fn rows(&self) -> Range<u16> {
        self.start..self.end
    }
}

/// What changed between two pictures. Buffers are kept across updates, so
/// computing damage allocates only when the screen grows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    full: bool,
    scrolls: Vec<Scroll>,
    dirty: Vec<u16>,
    /// Previous ids sorted, with their rows: a scratch index.
    index: Vec<(RowId, u16)>,
}

impl Damage {
    /// Whether everything must be repainted: first picture, other screen,
    /// other size. Every row is then also listed as dirty.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Rows moved unchanged, to apply before repainting the dirty rows.
    pub fn scrolls(&self) -> &[Scroll] {
        &self.scrolls
    }

    /// Rows to repaint, top to bottom.
    pub fn dirty(&self) -> &[u16] {
        &self.dirty
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        !self.full && self.scrolls.is_empty() && self.dirty.is_empty()
    }

    /// No damage, keeping the buffers.
    pub fn clear(&mut self) {
        self.full = false;
        self.scrolls.clear();
        self.dirty.clear();
    }

    /// The damage that turns picture `old` into `new`. `full` (or a change
    /// in row count) repaints everything.
    pub fn compute(&mut self, old: &[RowStamp], new: &[RowStamp], full: bool) {
        self.compute_with(old, new, full, |_, _| true);
    }

    /// [`Self::compute`] for pictures that hold more than the grid's rows:
    /// new row `r` may reuse old row `from` only if `same(r, from)` too.
    /// The frame passes whether the image slices on both rows match.
    pub fn compute_with(
        &mut self,
        old: &[RowStamp],
        new: &[RowStamp],
        full: bool,
        same: impl Fn(u16, u16) -> bool,
    ) {
        self.clear();
        let rows = (0..=u16::MAX).zip(new);
        if full || old.len() != new.len() {
            self.full = true;
            self.dirty.extend(rows.map(|(r, _)| r));
            return;
        }
        self.index.clear();
        self.index
            .extend(old.iter().map(|s| s.id).zip(0..=u16::MAX));
        self.index.sort_unstable();
        for (r, stamp) in rows {
            if old.get(usize::from(r)) == Some(stamp) && same(r, r) {
                continue;
            }
            match self.moved_from(old, *stamp).filter(|&from| same(r, from)) {
                Some(from) => self.push_scroll(r, from),
                None => self.dirty.push(r),
            }
        }
    }

    /// The previous row holding `stamp`, if it is there unchanged.
    fn moved_from(&self, old: &[RowStamp], stamp: RowStamp) -> Option<u16> {
        let at = self
            .index
            .binary_search_by_key(&stamp.id, |&(id, _)| id)
            .ok()?;
        let &(_, from) = self.index.get(at)?;
        (old.get(usize::from(from)) == Some(&stamp)).then_some(from)
    }

    /// Extends the last scroll when `r` continues it with the same offset.
    fn push_scroll(&mut self, r: u16, from: u16) {
        if let Some(last) = self.scrolls.last_mut()
            && last.end == r
            && u32::from(last.from) + u32::from(r - last.start) == u32::from(from)
        {
            last.end += 1;
            return;
        }
        self.scrolls.push(Scroll {
            start: r,
            end: r + 1,
            from,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(id: u64, generation: u32) -> RowStamp {
        RowStamp {
            id: RowId(id),
            generation,
        }
    }

    fn stamps(ids: &[u64]) -> Vec<RowStamp> {
        ids.iter().map(|&id| stamp(id, 0)).collect()
    }

    #[test]
    fn identical_pictures_have_no_damage() {
        let mut d = Damage::default();
        let s = stamps(&[1, 2, 3]);
        d.compute(&s, &s, false);
        assert!(d.is_empty());
    }

    #[test]
    fn a_full_or_resized_picture_repaints_every_row() {
        let mut d = Damage::default();
        let s = stamps(&[1, 2, 3]);
        d.compute(&s, &s, true);
        assert!(d.is_full());
        assert_eq!(d.dirty(), [0, 1, 2]);
        d.compute(&s[..2], &s, false);
        assert!(d.is_full(), "row count changed");
    }

    #[test]
    fn a_new_generation_is_dirty() {
        let mut d = Damage::default();
        let old = stamps(&[1, 2, 3]);
        let mut new = old.clone();
        new[1].generation = 7;
        d.compute(&old, &new, false);
        assert_eq!(d.dirty(), [1]);
        assert!(d.scrolls().is_empty());
    }

    #[test]
    fn a_scroll_up_is_one_move_and_a_new_bottom_row() {
        let mut d = Damage::default();
        d.compute(&stamps(&[1, 2, 3, 4]), &stamps(&[2, 3, 4, 5]), false);
        assert_eq!(
            d.scrolls(),
            [Scroll {
                start: 0,
                end: 3,
                from: 1
            }]
        );
        assert_eq!(d.dirty(), [3]);
    }

    #[test]
    fn rows_whose_extras_differ_are_neither_kept_nor_moved() {
        let mut d = Damage::default();
        d.compute_with(&stamps(&[1, 2, 3]), &stamps(&[1, 2, 3]), false, |r, _| {
            r != 1
        });
        assert_eq!(d.dirty(), [1]);
        let (old, new) = (stamps(&[1, 2, 3, 4]), stamps(&[2, 3, 4, 5]));
        d.compute_with(&old, &new, false, |r, from| (r, from) != (1, 2));
        assert_eq!(d.dirty(), [1, 3]);
        assert_eq!(d.scrolls().len(), 2, "the move splits around row 1");
    }

    #[test]
    fn a_moved_row_that_also_changed_is_repainted_not_moved() {
        let mut d = Damage::default();
        let old = stamps(&[1, 2, 3, 4]);
        let mut new = stamps(&[2, 3, 4, 5]);
        new[1].generation = 1;
        d.compute(&old, &new, false);
        assert_eq!(
            d.scrolls(),
            [
                Scroll {
                    start: 0,
                    end: 1,
                    from: 1
                },
                Scroll {
                    start: 2,
                    end: 3,
                    from: 3
                }
            ]
        );
        assert_eq!(d.dirty(), [1, 3]);
    }

    #[test]
    fn two_regions_scrolling_apart_are_two_moves() {
        let mut d = Damage::default();
        d.compute(
            &stamps(&[1, 2, 3, 4, 5, 6]),
            &stamps(&[2, 3, 9, 8, 4, 5]),
            false,
        );
        assert_eq!(
            d.scrolls(),
            [
                Scroll {
                    start: 0,
                    end: 2,
                    from: 1
                },
                Scroll {
                    start: 4,
                    end: 6,
                    from: 3
                }
            ]
        );
        assert_eq!(d.dirty(), [2, 3]);
    }
}
