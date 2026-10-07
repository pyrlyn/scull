//! Runs every line of the Unicode `GraphemeBreakTest.txt` through
//! `GraphemeState`. Kept apart from the unit tests because it reads the
//! committed conformance fixture that `scull-ucd-gen` copies from the pinned
//! data files.

use scull_unicode::{GraphemeState, UnicodeVersion};

const FIXTURE: &str = include_str!("data/GraphemeBreakTest.txt");
const BREAK: &str = "÷";
const NO_BREAK: &str = "×";

/// Code points and, per code point, whether a boundary precedes it; `None`
/// when the line holds something that is not a scalar value.
fn parse(line: &str) -> Option<Vec<(char, bool)>> {
    let mut out = Vec::new();
    let mut boundary = false;
    for token in line.split_whitespace() {
        match token {
            BREAK => boundary = true,
            NO_BREAK => boundary = false,
            hex => {
                let cp = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)?;
                out.push((cp, boundary));
            }
        }
    }
    Some(out)
}

#[test]
fn every_grapheme_break_test_line_passes() {
    let mut failures = Vec::new();
    let mut cases = 0;
    for (number, raw) in FIXTURE.lines().enumerate() {
        let line = raw.split_once('#').map_or(raw, |(data, _)| data).trim();
        if line.is_empty() {
            continue;
        }
        cases += 1;
        let mut state = GraphemeState::new(UnicodeVersion::LATEST);
        let Some(expected) = parse(line) else {
            failures.push(format!("line {} does not parse: {raw}", number + 1));
            continue;
        };
        let actual: Vec<(char, bool)> = expected
            .iter()
            .map(|&(cp, _)| (cp, state.next(cp)))
            .collect();
        if actual != expected {
            failures.push(format!("line {}: {raw}", number + 1));
        }
    }
    assert!(cases > 0, "fixture has no test cases");
    assert!(
        failures.is_empty(),
        "{} of {cases} lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
