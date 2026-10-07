//! Asserts the crate graph of `research.md` §7: dependencies point down, the
//! terminal state stays free of I/O, and only the C ABI reaches the PTY.

// Helpers outside #[test] functions are still test code; a failure here should abort loudly.
#![allow(clippy::unwrap_used)]

use workspace_graph::{Graph, Kind};

/// Edges between workspace crates, dev-dependencies excluded.
fn graph() -> Graph {
    Graph::load(env!("CARGO_MANIFEST_DIR"), &[Kind::Normal])
        .unwrap()
        .workspace_only()
}

#[test]
fn leaves_have_no_workspace_dependencies() {
    let graph = graph();
    for leaf in [
        "scull-unicode",
        "scull-input",
        "scull-pty",
        "scull-harness",
        "scull-image",
    ] {
        graph.assert_exact(leaf, &[]);
    }
}

#[test]
fn parser_depends_only_on_unicode() {
    graph().assert_exact("scull-parser", &["scull-unicode"]);
}

#[test]
fn grid_depends_only_on_unicode() {
    graph().assert_exact("scull-grid", &["scull-unicode"]);
}

#[test]
fn term_has_no_io_and_no_ffi() {
    graph().assert_forbidden("scull-term", &["scull-pty", "scull-ffi"]);
}

#[test]
fn nothing_depends_on_ffi() {
    graph().assert_only_dependents(&["scull-ffi"], &[]);
}
