use super::*;

fn term(cols: u16, rows: u16, feed: &str) -> Terminal {
    let mut t = Terminal::new(cols, rows, 50).unwrap();
    t.feed(feed.as_bytes());
    t
}

fn p(line: u64, col: u16) -> Point {
    Point { line, col }
}

fn m(start: (u64, u16), end: (u64, u16)) -> Match {
    Match {
        start: p(start.0, start.1),
        end: p(end.0, end.1),
    }
}

fn matches(t: &Terminal) -> Vec<Match> {
    let mut out = Vec::new();
    t.search_matches(&mut out);
    out
}

#[test]
fn case_folding_is_chosen_per_search() {
    let mut t = term(10, 3, "Foo foo\r\nFOO");
    assert!(t.search_set("foo", true));
    assert_eq!(matches(&t), [m((0, 4), (0, 6))]);
    assert!(t.search_set("fOo", false));
    assert_eq!(t.search_count(), 3);
    assert_eq!(matches(&t)[2], m((1, 0), (1, 2)));
}

#[test]
fn a_match_crosses_a_soft_wrap_but_not_a_hard_line_end() {
    let mut t = term(4, 4, "xxabcd\r\nab\r\ncd");
    assert!(t.search_set("abcd", true));
    assert_eq!(matches(&t), [m((0, 2), (1, 1))]);
    assert!(t.search_set("xx a", true));
    assert!(matches(&t).is_empty());
}

#[test]
fn a_wide_match_ends_on_its_spacer_and_padding_is_skipped() {
    let mut t = term(5, 2, "abcd\u{4e2d}\u{6587}");
    assert!(t.search_set("d\u{4e2d}\u{6587}", true));
    assert_eq!(matches(&t), [m((0, 3), (1, 3))]);
}

#[test]
fn matches_do_not_overlap_and_trailing_blanks_are_not_text() {
    let mut t = term(10, 2, "aaaaa\r\nb  ");
    assert!(t.search_set("aa", true));
    assert_eq!(matches(&t), [m((0, 0), (0, 1)), m((0, 2), (0, 3))]);
    assert!(t.search_set("b ", true));
    assert_eq!(t.search_count(), 0);
}

#[test]
fn history_is_searched_and_stepping_reveals_the_match() {
    let mut t = term(8, 2, "needle\r\n1\r\n2\r\n3\r\nneedle");
    assert!(t.search_set("needle", true));
    assert_eq!(matches(&t), [m((0, 0), (0, 5)), m((4, 0), (4, 5))]);
    let last = t.search_prev(None).unwrap();
    assert_eq!(last, m((4, 0), (4, 5)), "from the bottom of the viewport");
    assert_eq!(t.search_current(), Some(last));
    let first = t.search_prev(None).unwrap();
    assert_eq!(first.start.line, 0);
    assert_eq!(t.viewport_row(0), Some(0), "scrolled back to show it");
    assert_eq!(t.search_prev(None), Some(last), "wraps round");
    assert_eq!(t.search_next(None), Some(first), "wraps round");
    assert_eq!(t.search_next(Some(p(0, 0))), Some(last));
    assert_eq!(t.search_next(Some(p(4, 0))), Some(first));
}

#[test]
fn stepping_with_no_match_or_no_search_finds_nothing() {
    let mut t = term(8, 2, "abc");
    assert_eq!(t.search_next(None), None);
    assert!(t.search_set("zz", false));
    assert_eq!(t.search_next(None), None);
    assert_eq!(t.search_prev(None), None);
    assert_eq!(t.search_current(), None);
}

#[test]
fn a_backward_step_crosses_many_lines_back() {
    let mut text = String::from("hit");
    text.push_str(&"\r\n.".repeat(600));
    let mut t = Terminal::new(8, 4, 1000).unwrap();
    t.feed(text.as_bytes());
    assert!(t.search_set("hit", true));
    assert_eq!(t.search_prev(None), Some(m((0, 0), (0, 2))));
}

#[test]
fn bad_patterns_are_refused() {
    let mut t = term(8, 2, "abc");
    assert!(t.search_set("a", true));
    assert!(!t.search_set("", true));
    assert_eq!(t.search_count(), 0, "a refused pattern clears the search");
    assert!(!t.search_set(&"x".repeat(MAX_SEARCH_PATTERN + 1), true));
    assert!(t.search_set(&"x".repeat(MAX_SEARCH_PATTERN), true));
}

#[test]
fn a_long_pattern_matches_across_many_wrapped_rows() {
    let line = "0123456789".repeat(20);
    let mut t = term(7, 40, &format!("ab{line}"));
    assert!(t.search_set(&line, true));
    let found = matches(&t);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].start, p(0, 2));
    assert_eq!(found[0].end, p(28, 5), "202 cells end at line 28, column 5");
}

#[test]
fn counting_and_listing_stop_at_the_cap() {
    let mut t = Terminal::new(100, 120, 0).unwrap();
    t.feed(&b"a".repeat(12_000));
    assert!(t.search_set("a", true));
    assert_eq!(t.search_count(), MAX_SEARCH_MATCHES);
    assert_eq!(matches(&t).len(), MAX_SEARCH_MATCHES);
}

#[test]
fn a_resize_forgets_the_current_match_but_keeps_the_pattern() {
    let mut t = term(8, 2, "abc");
    assert!(t.search_set("b", true));
    assert!(t.search_next(None).is_some());
    t.resize(9, 2).unwrap();
    assert_eq!(t.search_current(), None);
    assert_eq!(t.search_count(), 1);
    t.search_clear();
    assert_eq!(t.search_count(), 0);
}
