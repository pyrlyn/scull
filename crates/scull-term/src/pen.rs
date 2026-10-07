//! The cursor's pen: the SGR style new cells are written with, and the grid
//! ids it was interned as. Separate because those ids live outside the grid:
//! a sweep may reclaim them, so each is tagged with the grid's
//! `intern_epoch` and re-interned when the epoch moves on.

use scull_grid::{Cell, Grid, Style, StyleId};

/// An interned id and the grid epoch it is valid in.
type Cached = Option<(StyleId, u64)>;

/// The current SGR style and its cached ids.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pen {
    style: Style,
    text: Cached,
    erase: Cached,
}

impl Pen {
    pub(crate) fn style(&self) -> &Style {
        &self.style
    }

    /// Changes the style; the cached ids no longer describe it.
    pub(crate) fn set(&mut self, style: Style) {
        if style != self.style {
            self.style = style;
            self.forget();
        }
    }

    /// Drops the cached ids, e.g. when another grid becomes active.
    pub(crate) fn forget(&mut self) {
        self.text = None;
        self.erase = None;
    }

    /// The id printed cells get. A full style table degrades to the default
    /// style rather than refusing the text.
    pub(crate) fn text_id(&mut self, grid: &mut Grid) -> StyleId {
        let style = self.style;
        lookup(&mut self.text, grid, &style)
    }

    /// What erasing leaves behind: blank cells with the pen's background
    /// only (background colour erase, as xterm and the VT220 do).
    pub(crate) fn blank(&mut self, grid: &mut Grid) -> Cell {
        let style = Style {
            bg: self.style.bg,
            ..Style::default()
        };
        if style == Style::default() {
            return Cell::EMPTY;
        }
        Cell::blank(lookup(&mut self.erase, grid, &style))
    }
}

fn lookup(cache: &mut Cached, grid: &mut Grid, style: &Style) -> StyleId {
    if let Some((id, epoch)) = *cache
        && epoch == grid.intern_epoch()
    {
        return id;
    }
    let id = grid.intern_style(style).unwrap_or(StyleId::DEFAULT);
    *cache = Some((id, grid.intern_epoch()));
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use scull_grid::{ClusterTable, Color, StyleTable};

    fn red(n: u8) -> Style {
        Style {
            fg: Color::Rgb(n, 0, 0),
            ..Style::default()
        }
    }

    #[test]
    fn pen_reinterns_after_a_sweep_reclaims_its_id() {
        const CAP: usize = 3;
        let mut grid = Grid::new(4, 2, 0)
            .unwrap()
            .with_tables(StyleTable::with_cap(CAP), ClusterTable::default());
        let mut pen = Pen::default();
        pen.set(red(1));
        let first = pen.text_id(&mut grid);
        // Two more styles fill the table; the next one sweeps the pen's
        // unused id away and the epoch moves on.
        grid.intern_style(&red(2)).unwrap();
        grid.intern_style(&red(3)).unwrap();
        assert_eq!(grid.intern_epoch(), 1);
        assert_eq!(grid.styles().get(first), Some(&red(3)), "id reused");
        let again = pen.text_id(&mut grid);
        assert_eq!(grid.styles().get(again), Some(&red(1)));
    }

    #[test]
    fn full_style_table_degrades_to_the_default_style() {
        const CAP: usize = 1;
        let mut grid = Grid::new(4, 2, 0)
            .unwrap()
            .with_tables(StyleTable::with_cap(CAP), ClusterTable::default());
        let mut pen = Pen::default();
        pen.set(red(1));
        assert_eq!(pen.text_id(&mut grid), StyleId::DEFAULT);
    }

    #[test]
    fn erase_keeps_only_the_background() {
        let mut grid = Grid::new(4, 2, 0).unwrap();
        let mut pen = Pen::default();
        pen.set(red(1));
        assert_eq!(pen.blank(&mut grid), Cell::EMPTY);
        pen.set(Style {
            bg: Color::Indexed(4),
            ..red(1)
        });
        let blank = pen.blank(&mut grid);
        let style = grid.styles().get(blank.style()).unwrap();
        assert_eq!((style.bg, style.fg), (Color::Indexed(4), Color::Default));
    }
}
