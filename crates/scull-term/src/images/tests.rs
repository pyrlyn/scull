//! Golden tests for the image protocols through `Terminal::feed`: where the
//! cursor ends, where the image is placed, what is replied, and how
//! placements follow scrolling, eviction, reflow and screen switches.

use crate::Terminal;

/// A 2 x 6 sixel: one red column, then one unpainted column.
const SIXEL: &[u8] = b"\x1bPq#0;2;100;0;0#0~?\x1b\\";
/// The same with `P2 = 1`: unpainted pixels stay transparent.
const SIXEL_CLEAR: &[u8] = b"\x1bP0;1q#0;2;100;0;0#0~?\x1b\\";
/// A 1 x 1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
/// kitty: transmit and display image 1, 2 x 2 RGB pixels.
const KITTY: &[u8] = b"\x1b_Gi=1,f=24,s=2,v=2,a=T;AAAAAAAAAAAAAAAA\x1b\\";
const KITTY_OK: &[u8] = b"\x1b_Gi=1;OK\x1b\\";

const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [0; 4];

fn term(cols: u16, rows: u16, scrollback: usize) -> Terminal {
    Terminal::new(cols, rows, scrollback).unwrap()
}

fn iterm(args: &str) -> Vec<u8> {
    format!("\x1b]1337;File=inline=1{args}:{PNG}\x07").into_bytes()
}

/// Each placement as (screen row, col, cols, rows); negative rows are above
/// the screen.
fn placed(t: &Terminal) -> Vec<(i64, u32, u32, u32)> {
    let top = i64::try_from(t.screen_top_line()).unwrap();
    t.images()
        .placements()
        .iter()
        .map(|p| (i64::try_from(p.row).unwrap() - top, p.col, p.cols, p.rows))
        .collect()
}

fn at(t: &Terminal) -> (u16, u16) {
    (t.cursor().row, t.cursor().col)
}

/// The pixel at `(x, y)` of the only image.
fn pixel(t: &mut Terminal, x: usize, y: usize) -> [u8; 4] {
    let id = t.images().placements()[0].image;
    let image = t.images().peek(id).unwrap();
    let i = y * image.stride() + x * 4;
    image.pixels()[i..i + 4].try_into().unwrap()
}

#[test]
fn sixel_leaves_the_cursor_under_the_image_left_edge() {
    let mut t = term(10, 6, 0);
    t.set_cell_size(2, 3);
    t.feed(b"\x1b[2;3H");
    t.feed(SIXEL);
    assert_eq!(placed(&t), [(1, 2, 1, 2)]);
    assert_eq!(at(&t), (3, 2));
    assert!(t.take_replies().is_empty());
}

#[test]
fn sixel_at_the_bottom_scrolls_and_the_image_keeps_its_line() {
    let mut t = term(10, 4, 10);
    t.set_cell_size(2, 3);
    t.feed(b"\x1b[4H");
    t.feed(SIXEL);
    assert_eq!(at(&t), (3, 0));
    assert_eq!(placed(&t), [(1, 0, 1, 2)], "two scrolls lifted it");
}

#[test]
fn sixel_background_is_the_terminal_s_unless_p2_says_transparent() {
    let mut t = term(10, 4, 0);
    t.set_cell_size(4, 8);
    t.set_background([1, 2, 3]);
    t.feed(SIXEL);
    assert_eq!(pixel(&mut t, 0, 0), RED);
    assert_eq!(pixel(&mut t, 1, 0), [1, 2, 3, 255]);
    assert_eq!(pixel(&mut t, 3, 7), CLEAR, "padding to the cell is clear");

    let mut t = term(10, 4, 0);
    t.feed(SIXEL_CLEAR);
    assert_eq!(pixel(&mut t, 0, 0), RED);
    assert_eq!(pixel(&mut t, 1, 0), CLEAR);
}

#[test]
fn a_cancelled_sixel_shows_nothing_and_leaves_the_cursor() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1bPq#0;2;100;0;0#0~~\x18");
    assert!(t.images().is_empty());
    assert_eq!(at(&t), (0, 0));
}

#[test]
fn a_dcs_that_is_not_sixel_is_ignored() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1bP$q#0~~\x1b\\\x1bP?1q~~\x1b\\");
    assert!(t.images().is_empty());
}

#[test]
fn an_image_drawn_over_another_replaces_it() {
    let mut t = term(10, 4, 0);
    for _ in 0..5 {
        t.feed(b"\x1b[H");
        t.feed(SIXEL);
    }
    assert_eq!(placed(&t), [(0, 0, 1, 1)]);
    assert_eq!(t.images().len(), 1, "the covered images are freed");
}

#[test]
fn iterm_leaves_the_cursor_right_of_the_image() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1b[2;3H");
    t.feed(&iterm(""));
    assert_eq!(placed(&t), [(1, 2, 1, 1)]);
    assert_eq!(at(&t), (1, 3));
}

#[test]
fn iterm_can_keep_the_cursor_still() {
    let mut t = term(10, 4, 0);
    t.feed(&iterm(";doNotMoveCursor=1"));
    assert_eq!(placed(&t), [(0, 0, 1, 1)]);
    assert_eq!(at(&t), (0, 0));
}

#[test]
fn iterm_sizes_in_cells_ignore_the_aspect_when_asked() {
    let mut t = term(10, 4, 0);
    t.feed(&iterm(";width=3;height=2;preserveAspectRatio=0"));
    assert_eq!(placed(&t), [(0, 0, 3, 2)]);
    assert_eq!(at(&t), (1, 3));
}

#[test]
fn iterm_fits_the_image_inside_both_sizes_by_default() {
    let mut t = term(10, 4, 0);
    t.feed(&iterm(";width=3;height=2"));
    // 3 x 2 cells of 8 x 16 px: the square fits as 24 x 24 px.
    assert_eq!(placed(&t), [(0, 0, 3, 2)]);
    let mut t = term(10, 4, 0);
    t.feed(&iterm(";width=4;height=1"));
    assert_eq!(placed(&t), [(0, 0, 2, 1)], "16 px tall, so 16 px wide");
}

#[test]
fn iterm_percent_and_pixel_sizes_turn_into_cells() {
    let mut t = term(10, 4, 0);
    // Half of 80 px is 40 px: 5 columns, and a square 40 px is 3 rows.
    t.feed(&iterm(";width=50%"));
    assert_eq!(placed(&t), [(0, 0, 5, 3)]);
    let mut t = term(10, 4, 0);
    t.feed(&iterm(";height=17px"));
    assert_eq!(placed(&t), [(0, 0, 3, 2)]);
}

#[test]
fn iterm_downloads_and_other_osc_1337_are_ignored() {
    let mut t = term(10, 4, 0);
    t.feed(format!("\x1b]1337;File=name=eA==:{PNG}\x07").as_bytes());
    t.feed(b"\x1b]1337;SetMark\x07\x1b]2;title\x07");
    assert!(t.images().is_empty());
    assert_eq!(at(&t), (0, 0));
}

#[test]
fn the_cell_size_turns_pixels_into_cells_and_rejects_nonsense() {
    let mut t = term(10, 4, 0);
    assert_eq!(t.cell_size(), crate::DEFAULT_CELL_PX);
    t.set_cell_size(0, 10);
    t.set_cell_size(10, crate::MAX_CELL_PX + 1);
    assert_eq!(t.cell_size(), crate::DEFAULT_CELL_PX);
    t.set_cell_size(1, 1);
    t.feed(&iterm(";width=3px;height=2px;preserveAspectRatio=0"));
    assert_eq!(placed(&t), [(0, 0, 3, 2)]);
}

#[test]
fn kitty_replies_ok_and_moves_the_cursor_past_the_image() {
    let mut t = term(10, 4, 0);
    t.feed(KITTY);
    assert_eq!(t.take_replies(), KITTY_OK);
    assert_eq!(placed(&t), [(0, 0, 1, 1)]);
    assert_eq!(at(&t), (0, 1));
}

#[test]
fn kitty_c1_keeps_the_cursor_still() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1b_Gi=1,f=24,s=2,v=2,a=T,C=1,c=3,r=2;AAAAAAAAAAAAAAAA\x1b\\");
    assert_eq!(placed(&t), [(0, 0, 3, 2)]);
    assert_eq!(at(&t), (0, 0));
}

#[test]
fn kitty_sized_placement_moves_the_cursor_to_its_last_row() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1b_Gi=1,f=24,s=2,v=2,a=T,c=3,r=2,q=1;AAAAAAAAAAAAAAAA\x1b\\");
    assert!(t.take_replies().is_empty(), "q=1 hides OK");
    assert_eq!(at(&t), (1, 3));
}

#[test]
fn kitty_errors_are_replied_and_place_nothing() {
    let mut t = term(10, 4, 0);
    t.feed(b"\x1b_Gi=7,f=24,s=2,v=2,a=T;AAAA\x1b\\");
    let reply = t.take_replies();
    assert!(reply.starts_with(b"\x1b_Gi=7;E"), "{reply:?}");
    assert!(t.images().placements().is_empty());
    assert_eq!(at(&t), (0, 0));
}

#[test]
fn kitty_delete_removes_its_placements() {
    let mut t = term(10, 4, 0);
    t.feed(KITTY);
    t.feed(b"\x1b_Ga=d\x1b\\");
    assert!(placed(&t).is_empty());
    t.feed(b"\x1b_Ga=p,i=1,q=2\x1b\\");
    assert_eq!(
        placed(&t),
        [(0, 1, 1, 1)],
        "the image is kept to place again"
    );
}

#[test]
fn a_flood_of_kitty_replies_stays_within_the_queue_cap() {
    let mut t = term(10, 4, 0);
    t.feed(&b"\x1b_Gi=9,a=q,s=1,v=1,f=24;AAAA\x1b\\".repeat(2000));
    let replies = t.take_replies();
    assert!(!replies.is_empty());
    assert!(replies.len() <= crate::reply::MAX_REPLY_BYTES);
    assert!(replies.ends_with(b"\x1b\\"), "only whole replies are kept");
}

#[test]
fn images_keep_their_line_while_the_text_scrolls() {
    let mut t = term(10, 4, 10);
    t.feed(b"\x1b[2H");
    t.feed(&iterm(""));
    t.feed(b"\r\n\r\n\r\n\r\n");
    assert_eq!(
        placed(&t),
        [(-1, 0, 1, 1)],
        "two of the line feeds scrolled"
    );
    t.feed(b"\x1b[S");
    assert_eq!(placed(&t), [(-2, 0, 1, 1)], "SU scrolls the image away too");
}

#[test]
fn an_image_evicted_from_history_is_dropped() {
    let mut t = term(10, 2, 2);
    t.feed(&iterm(";doNotMoveCursor=1"));
    t.feed(b"\r\n\r\n\r\n");
    assert_eq!(placed(&t), [(-2, 0, 1, 1)], "still in history");
    t.feed(b"\r\n");
    assert!(t.images().is_empty(), "the image went with its line");
}

#[test]
fn a_tall_image_stays_while_its_bottom_is_kept() {
    let mut t = term(10, 2, 1);
    t.feed(b"\x1b_Gi=1,f=24,s=2,v=2,a=T,C=1,c=1,r=2,q=2;AAAAAAAAAAAAAAAA\x1b\\");
    t.feed(b"\r\n\r\n");
    assert_eq!(placed(&t), [(-1, 0, 1, 2)]);
    t.feed(b"\r\n");
    assert_eq!(placed(&t), [(-2, 0, 1, 2)], "its top left the ring");
    t.feed(b"\r\n");
    assert!(placed(&t).is_empty());
    assert_eq!(t.images().len(), 1, "kitty images outlive placements");
}

#[test]
fn erasing_the_history_drops_its_images_and_keeps_the_screen_s() {
    let mut t = term(10, 3, 10);
    t.feed(&iterm(""));
    t.feed(b"\r\n\r\n\r\n");
    t.feed(&iterm(""));
    assert_eq!(placed(&t), [(-1, 0, 1, 1), (2, 0, 1, 1)]);
    t.feed(b"\x1b[3J");
    assert_eq!(placed(&t), [(2, 0, 1, 1)]);
    assert_eq!(t.images().len(), 1);
}

#[test]
fn erasing_the_screen_drops_its_images_and_keeps_the_history_s() {
    let mut t = term(10, 3, 10);
    t.feed(&iterm(""));
    t.feed(b"\r\n\r\n\r\n");
    t.feed(&iterm(""));
    t.feed(b"\x1b[2J");
    assert_eq!(placed(&t), [(-1, 0, 1, 1)]);
    t.feed(b"\x1b[H");
    t.feed(&iterm(""));
    t.feed(b"\x1b[1J");
    assert_eq!(placed(&t).len(), 2, "only ED 2 takes images");
}

#[test]
fn leaving_the_alternate_screen_drops_its_images() {
    let mut t = term(10, 3, 10);
    t.feed(&iterm(""));
    t.feed(b"\x1b[?1049h");
    assert!(placed(&t).is_empty(), "each screen shows its own");
    t.feed(b"\x1b[2;2H");
    t.feed(&iterm(""));
    t.feed(KITTY);
    assert_eq!(placed(&t).len(), 2);
    t.feed(b"\x1b[?1049l");
    assert_eq!(
        placed(&t),
        [(0, 0, 1, 1)],
        "the main screen's image is back"
    );
    t.feed(b"\x1b[?1049h");
    assert!(t.images().is_empty(), "the alternate screen starts empty");
    t.feed(b"\x1b_Ga=p,i=1,q=2\x1b\\");
    assert!(placed(&t).is_empty(), "kitty's image went too");
}

#[test]
fn a_reset_drops_every_image() {
    let mut t = term(10, 3, 10);
    t.feed(&iterm(""));
    t.feed(KITTY);
    t.feed(b"\x1bPq#0~");
    t.feed(b"\x1bc");
    assert!(t.images().is_empty());
    t.feed(b"~\x1b\\");
    assert!(t.images().is_empty(), "the sixel cut by RIS is gone");
}
