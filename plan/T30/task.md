# T30. Config watcher never misses a reload under load

Five live-reload tests (`scull-config` `live_tests.rs`, `scull-ffi` `config.rs`) time out under heavy load. Cause, reproduced with a traced watcher: when its client queue overflows, FSEvents drops the queued events and reports one `Rescan` event naming the watched directory ("rescan: user dropped"); the watcher only reacts to events naming `config.toml`, so the lost write is never reloaded. A second flake: an in-place write (truncate, then write) read in between yields an empty file, so a test that asserts the last good settings after a broken edit can see the defaults. Done when a dropped event still leads to a reload and the tests stop depending on scheduling.

Execution plan:
1. `crates/scull-config/src/live.rs`: treat a rescan event as relevant (filter extracted into a function with a unit test).
2. Same file: a resync on the watcher thread; with no event for a while it compares the file's modification time and length with the last reload and reloads on a difference, which covers drops a backend does not report. The stamp is taken before the first load, so a write between the load and the watch start is not lost either.
3. Tests that assert the last good settings after a broken edit replace the file by rename, as editors do, so no half-written file is read.
4. Verify: a temporary probe (flooding the directory with a slowed FSEvents client) fails before and passes after; the five tests under `cargo nextest --stress-count`; `just check`.
