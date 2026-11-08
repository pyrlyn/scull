# T22. No-op link clears are generation-neutral in Row::set_link

`crates/scull-grid/src/row.rs`'s `set_link` bumped the generation unconditionally, so clearing links on every plain cell write forced a repaint per write — while `set()` deliberately skips the bump for no-op writes. Found by the 2026-10-07 audit.
