//! Divan bench of a resize that rewraps a full scrollback: 10,000 history
//! rows at 120 columns reflowed to 80 and back, the cost a window drag pays
//! once at its end (`cargo bench -p scull-grid --bench reflow`).
//!
//! Recorded 2026-10-07, Apple Silicon, rustc 1.99.0: both passes together
//! take a median of 6.3 ms (fastest 6.0 ms, slowest 7.7 ms over 100 samples).

#![allow(clippy::expect_used, reason = "a bench aborts on a broken fixture")]

use divan::Bencher;
use scull_grid::{Cell, Grid, Reflow, StyleId, TrackPoint};

const WIDE_COLS: u16 = 120;
const NARROW_COLS: u16 = 80;
const SCREEN: u16 = 40;
const HISTORY: usize = 10_000;
/// Line lengths cycled through: short output, a typical log line, one that
/// just fits, and one that wraps at 120 columns.
const LINE_LENS: [u16; 4] = [20, 72, 119, 200];

fn main() {
    divan::main();
}

/// A grid whose history is full of plain lines, some of them soft-wrapped.
fn full_grid() -> Grid {
    let mut g = Grid::new(WIDE_COLS, SCREEN, HISTORY).expect("bench grid");
    let bottom = SCREEN - 1;
    for len in LINE_LENS.iter().cycle() {
        if g.history_len() >= HISTORY {
            break;
        }
        let mut col = 0;
        for ch in ('a'..='z').cycle().take(usize::from(*len)) {
            let row = g.screen_row_mut(bottom).expect("bottom row");
            if col == WIDE_COLS {
                row.set_wrapped(true);
                g.scroll_up(1, Cell::EMPTY);
                col = 0;
            }
            let row = g.screen_row_mut(bottom).expect("bottom row");
            row.put(col, Cell::char(ch, StyleId::DEFAULT), false)
                .expect("in row");
            col += 1;
        }
        g.scroll_up(1, Cell::EMPTY);
    }
    g
}

/// Rewrap the whole history to 80 columns and back to 120.
#[divan::bench]
fn reflow_history_120_80_120(bencher: Bencher) {
    let g = full_grid();
    bencher
        .with_inputs(|| g.clone())
        .bench_local_values(|mut g| {
            let mut cursor = TrackPoint::new(g.history_len(), 0);
            for cols in [NARROW_COLS, WIDE_COLS] {
                g.resize(cols, SCREEN, Reflow::Rewrap, &mut cursor, &mut [])
                    .expect("resize");
            }
            g
        });
}
