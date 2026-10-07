//! Divan benches of the scrollback ring, plus the memory one history row
//! costs. `cargo bench -p scull-grid --bench scrollback` prints bytes per
//! history row (row header plus stored cells) before the timings; the divan
//! allocation profiler adds allocations per scrolled line.
//!
//! Recorded 2026-10-07, Apple Silicon, rustc 1.99.0, 120 columns, 10,000
//! history rows: blank row 56 B, plain text row (72 cells) 632 B, SGR-heavy
//! row (a style change in every one of 120 cells, 33 styles in the table)
//! 1016 B. Medians: `scroll_full_ring` 25-66 ns across runs with no
//! allocation (one free: the evicted row's cells),
//! `write_and_scroll_plain_row` 513 ns, `write_and_scroll_sgr_row` 388 ns.

#![allow(
    clippy::print_stdout,
    reason = "the bench reports bytes per scrollback row"
)]
#![allow(clippy::expect_used, reason = "a bench aborts on a broken fixture")]

use divan::{AllocProfiler, Bencher};
use scull_grid::{Attrs, Cell, Color, Grid, Style, StyleId};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

const COLS: u16 = 120;
const SCREEN: u16 = 40;
const HISTORY: usize = 10_000;
/// A typical log line: shorter than the window, default style.
const PLAIN_LEN: u16 = 72;
/// Palette colours the SGR-heavy row cycles through, each plain and bold.
const PALETTE: u8 = 16;

fn grid() -> Grid {
    Grid::new(COLS, SCREEN, HISTORY).expect("bench grid")
}

fn bottom(g: &mut Grid) -> &mut scull_grid::Row {
    g.screen_row_mut(SCREEN - 1).expect("bottom row")
}

fn plain_line(g: &mut Grid) {
    let row = bottom(g);
    for (col, ch) in (0..PLAIN_LEN).zip(('a'..='z').cycle()) {
        row.put(col, Cell::char(ch, StyleId::DEFAULT), false)
            .expect("in row");
    }
}

fn sgr_styles(g: &mut Grid) -> Vec<StyleId> {
    let mut ids = Vec::new();
    for n in 0..PALETTE {
        for attrs in [Attrs::empty(), Attrs::BOLD] {
            let style = Style {
                fg: Color::Indexed(n),
                bg: Color::Indexed(PALETTE - 1 - n),
                attrs,
                ..Style::default()
            };
            ids.push(g.intern_style(&style).expect("style"));
        }
    }
    ids
}

fn sgr_line(g: &mut Grid, styles: &[StyleId]) {
    let row = bottom(g);
    for (col, style) in (0..COLS).zip(styles.iter().cycle()) {
        row.put(col, Cell::char('x', *style), false)
            .expect("in row");
    }
}

fn fill_history(mut line: impl FnMut(&mut Grid), g: &mut Grid) {
    for _ in 0..HISTORY + usize::from(SCREEN) {
        line(g);
        g.scroll_up(1, Cell::EMPTY);
    }
}

fn bytes_per_history_row(g: &Grid) -> usize {
    let rows = g.history_len().max(1);
    let total: usize = (0..g.history_len())
        .filter_map(|i| g.row(i))
        .map(scull_grid::Row::memory_bytes)
        .sum();
    total / rows
}

fn report() {
    let mut blank = grid();
    fill_history(|_| {}, &mut blank);
    let mut plain = grid();
    fill_history(plain_line, &mut plain);
    let mut sgr = grid();
    let styles = sgr_styles(&mut sgr);
    fill_history(|g| sgr_line(g, &styles), &mut sgr);
    println!(
        "bytes per scrollback row at {COLS} columns: blank {}, plain text ({PLAIN_LEN} cells) {}, \
         SGR-heavy ({COLS} cells, {} styles) {}",
        bytes_per_history_row(&blank),
        bytes_per_history_row(&plain),
        sgr.styles().len(),
        bytes_per_history_row(&sgr),
    );
}

fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        report();
    }
    divan::main();
}

/// One line into a full ring: offset change plus a recycled row.
#[divan::bench]
fn scroll_full_ring(bencher: Bencher) {
    let mut g = grid();
    fill_history(plain_line, &mut g);
    bencher.bench_local(|| g.scroll_up(1, Cell::EMPTY));
}

/// Write a plain line, then scroll it into a full history.
#[divan::bench]
fn write_and_scroll_plain_row(bencher: Bencher) {
    let mut g = grid();
    fill_history(plain_line, &mut g);
    bencher.bench_local(|| {
        plain_line(&mut g);
        g.scroll_up(1, Cell::EMPTY);
    });
}

/// Write an SGR-heavy line, then scroll it into a full history.
#[divan::bench]
fn write_and_scroll_sgr_row(bencher: Bencher) {
    let mut g = grid();
    let styles = sgr_styles(&mut g);
    fill_history(|g| sgr_line(g, &styles), &mut g);
    bencher.bench_local(|| {
        sgr_line(&mut g, &styles);
        g.scroll_up(1, Cell::EMPTY);
    });
}
