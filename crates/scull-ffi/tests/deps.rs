//! Asserts the crate graph of `research.md` §7: dependencies point down, the
//! terminal state stays free of I/O, and only the C ABI reaches the PTY.

// Helpers outside #[test] functions are still test code; a failure here should abort loudly.
#![allow(clippy::unwrap_used, clippy::panic)]

use cargo_metadata::{CargoOpt, Metadata, MetadataCommand};

fn metadata() -> Metadata {
    MetadataCommand::new()
        .features(CargoOpt::AllFeatures)
        .exec()
        .unwrap()
}

/// Workspace crates the named crate depends on, dev-dependencies excluded.
fn workspace_deps(meta: &Metadata, name: &str) -> Vec<String> {
    let pkg = meta
        .workspace_packages()
        .into_iter()
        .find(|p| p.name.as_str() == name)
        .unwrap_or_else(|| panic!("{name} is not a workspace member"));
    pkg.dependencies
        .iter()
        .filter(|d| d.kind == cargo_metadata::DependencyKind::Normal)
        .filter(|d| d.name.starts_with("scull-"))
        .map(|d| d.name.clone())
        .collect()
}

#[test]
fn leaves_have_no_workspace_dependencies() {
    let meta = metadata();
    for leaf in [
        "scull-unicode",
        "scull-input",
        "scull-pty",
        "scull-harness",
        "scull-image",
    ] {
        assert_eq!(workspace_deps(&meta, leaf), Vec::<String>::new(), "{leaf}");
    }
}

#[test]
fn parser_depends_only_on_unicode() {
    assert_eq!(
        workspace_deps(&metadata(), "scull-parser"),
        ["scull-unicode"]
    );
}

#[test]
fn grid_depends_only_on_unicode() {
    assert_eq!(workspace_deps(&metadata(), "scull-grid"), ["scull-unicode"]);
}

#[test]
fn term_has_no_io_and_no_ffi() {
    let deps = workspace_deps(&metadata(), "scull-term");
    for banned in ["scull-pty", "scull-ffi"] {
        assert!(
            !deps.iter().any(|d| d == banned),
            "scull-term depends on {banned}"
        );
    }
}

#[test]
fn nothing_depends_on_ffi() {
    let meta = metadata();
    for pkg in meta.workspace_packages() {
        assert!(
            !workspace_deps(&meta, pkg.name.as_str())
                .iter()
                .any(|d| d == "scull-ffi"),
            "{} depends on scull-ffi",
            pkg.name
        );
    }
}
