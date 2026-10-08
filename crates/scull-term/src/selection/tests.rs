use super::*;

fn term(cols: u16, rows: u16, feed: &str) -> Terminal {
    let mut t = Terminal::new(cols, rows, 20).unwrap();
    t.feed(feed.as_bytes());
    t
}

/// Selects from viewport cell `a` to `b` and reads the text.
fn select(t: &mut Terminal, kind: SelectionKind, a: (u16, u16), b: (u16, u16)) -> String {
    let (a, b) = (t.viewport_point(a.0, a.1), t.viewport_point(b.0, b.1));
    t.select_start(kind, a);
    t.select_extend(b);
    text(t)
}

fn text(t: &Terminal) -> String {
    let mut out = String::new();
    t.selection_text(&mut out);
    out
}

#[test]
fn a_cell_selection_trims_each_line_and_ends_it_with_a_newline() {
    let mut t = term(8, 3, "ab  \r\nxy z\r\n12");
    assert_eq!(
        select(&mut t, SelectionKind::Cell, (0, 1), (2, 0)),
        "b\nxy z\n1"
    );
    assert_eq!(
        select(&mut t, SelectionKind::Cell, (1, 3), (0, 6)),
        "\nxy z",
        "backwards"
    );
}

#[test]
fn a_soft_wrap_joins_its_rows_and_a_blank_selection_reads_empty() {
    let mut t = term(4, 3, "abcdefg");
    assert_eq!(select(&mut t, SelectionKind::Cell, (0, 2), (1, 3)), "cdefg");
    assert_eq!(select(&mut t, SelectionKind::Cell, (2, 0), (2, 3)), "");
}

#[test]
fn wide_characters_read_once_from_either_half() {
    let mut t = term(8, 2, "a\u{4e2d}b");
    assert_eq!(
        select(&mut t, SelectionKind::Cell, (0, 2), (0, 2)),
        "\u{4e2d}"
    );
    let (_, start, end) = t.selection().unwrap();
    assert_eq!((start.col, end.col), (1, 2), "the spacer pulls in its head");
    assert_eq!(
        select(&mut t, SelectionKind::Cell, (0, 0), (0, 1)),
        "a\u{4e2d}"
    );
}

#[test]
fn a_wide_character_wrapped_early_skips_its_leading_padding() {
    let mut t = term(5, 2, "abcd\u{4e2d}");
    assert_eq!(
        select(&mut t, SelectionKind::Cell, (0, 0), (1, 1)),
        "abcd\u{4e2d}"
    );
    assert_eq!(
        select(&mut t, SelectionKind::Word, (1, 0), (1, 0)),
        "abcd\u{4e2d}"
    );
}

#[test]
fn a_word_ends_at_blanks_and_separators() {
    let mut t = term(30, 2, "ls ~/a-b/c.txt (x)  y");
    let word = |t: &mut Terminal, col| select(t, SelectionKind::Word, (0, col), (0, col));
    assert_eq!(word(&mut t, 6), "~/a-b/c.txt");
    assert_eq!(word(&mut t, 15), "(", "a separator is a word of its own");
    assert_eq!(word(&mut t, 16), "x");
    assert_eq!(word(&mut t, 19), "", "a run of blanks trims to nothing");
    assert_eq!(
        select(&mut t, SelectionKind::Word, (0, 1), (0, 3)),
        "ls ~/a-b/c.txt"
    );
}

#[test]
fn a_word_runs_across_a_soft_wrap_but_not_a_hard_line_end() {
    let mut t = term(4, 3, "x abcdef\r\ngh");
    assert_eq!(
        select(&mut t, SelectionKind::Word, (1, 1), (1, 1)),
        "abcdef"
    );
    assert_eq!(select(&mut t, SelectionKind::Word, (2, 0), (2, 0)), "gh");
}

#[test]
fn a_line_selection_takes_the_whole_wrapped_line() {
    let mut t = term(4, 4, "abcdef\r\nxy\r\nz");
    assert_eq!(
        select(&mut t, SelectionKind::Line, (1, 1), (1, 1)),
        "abcdef"
    );
    assert_eq!(
        select(&mut t, SelectionKind::Line, (0, 3), (2, 0)),
        "abcdef\nxy"
    );
}

#[test]
fn a_block_takes_the_same_columns_of_every_row() {
    let mut t = term(6, 3, "abcdef\r\nghijkl\r\nmn");
    assert_eq!(
        select(&mut t, SelectionKind::Block, (0, 4), (2, 1)),
        "bcde\nhijk\nn"
    );
}

#[test]
fn concealed_text_is_copied_as_blanks() {
    let mut t = term(12, 1, "pw \x1b[8mhunter2\x1b[m!");
    assert_eq!(
        select(&mut t, SelectionKind::Line, (0, 0), (0, 0)),
        "pw        !"
    );
}

#[test]
fn the_selection_stays_on_its_text_while_output_scrolls() {
    let mut t = term(6, 3, "one\r\ntwo\r\n");
    assert_eq!(
        select(&mut t, SelectionKind::Line, (0, 0), (1, 0)),
        "one\ntwo"
    );
    t.feed(b"3\r\n4\r\n5\r\n6");
    assert_eq!(text(&t), "one\ntwo", "scrolled into history");
    t.scroll_display(4);
    assert_eq!(t.viewport_row(t.selection().unwrap().1.line), Some(0));
    t.scroll_display(-4);
    assert_eq!(text(&t), "one\ntwo", "the viewport is not the selection");
}

#[test]
fn writing_into_a_selected_row_drops_the_selection_and_elsewhere_does_not() {
    let mut t = term(6, 3, "one\r\ntwo\r\nend");
    select(&mut t, SelectionKind::Cell, (0, 0), (0, 2));
    t.feed(b"\x1b[3;1Hxyz");
    assert_eq!(text(&t), "one", "another row changed");
    t.feed(b"\x1b[1;2HX");
    assert!(t.selection().is_none());
    select(&mut t, SelectionKind::Cell, (0, 0), (1, 2));
    t.feed(b"\x1b[2;3r\x1b[2;1H\x1bM");
    assert!(t.selection().is_none(), "a region scroll moved a row away");
}

#[test]
fn a_resize_a_screen_switch_and_eviction_drop_the_selection() {
    // Three rows and a fourth line, so "one" is in the history.
    let mut t = term(6, 3, "one\r\ntwo\r\n3\r\n4");
    let drops = |t: &mut Terminal, change: &dyn Fn(&mut Terminal)| {
        select(t, SelectionKind::Line, (0, 0), (0, 0));
        assert!(t.selection().is_some());
        change(t);
        t.selection().is_none()
    };
    assert!(!drops(&mut t, &|t| t.resize(6, 3).unwrap()), "same size");
    assert!(drops(&mut t, &|t| t.resize(7, 3).unwrap()));
    assert!(drops(&mut t, &|t| t.feed(b"\x1b[?1049h")));
    t.feed(b"\x1b[?1049l");
    assert!(
        !drops(&mut t, &|t| t.feed(b"\x1b[3J")),
        "a screen row stays"
    );
    t.feed(b"\r\n5");
    t.scroll_display(1);
    assert!(
        drops(&mut t, &|t| t.feed(b"\x1b[3J")),
        "its history row went"
    );
    assert!(
        drops(&mut t, &|t| t.feed(&b"\n".repeat(40))),
        "scrolled out of the ring"
    );
}

#[test]
fn points_outside_the_ring_select_nothing_or_clamp() {
    let mut t = term(6, 3, "ab");
    t.select_start(SelectionKind::Cell, Point { line: 99, col: 0 });
    assert!(t.selection().is_none());
    t.select_start(SelectionKind::Cell, t.viewport_point(0, 1));
    t.select_extend(Point {
        line: u64::MAX,
        col: u16::MAX,
    });
    assert_eq!(text(&t), "b\n\n");
    t.select_clear();
    assert_eq!(text(&t), "");
}

#[test]
fn the_text_is_cut_at_the_cap_on_a_character_boundary() {
    let mut t = term(8, 1, "a\u{e9}\u{e9}");
    select(&mut t, SelectionKind::Line, (0, 0), (0, 0));
    let mut out = String::new();
    t.selection_text_capped(&mut out, 2);
    assert_eq!(out, "a", "half of \u{e9} is not kept");
    out.clear();
    t.selection_text_capped(&mut out, 3);
    assert_eq!(out, "a\u{e9}");
}
