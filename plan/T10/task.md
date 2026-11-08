# T10. C ABI

`scull-ffi`: opaque handles, the frame API, one wakeup callback, a polled event queue, status codes, `catch_unwind` on every export with per-terminal poisoning, an ABI version check and `struct_size` in every struct. The header and C# bindings are generated, committed and diff-checked in CI. Done when a C test program runs create, feed, update and free under the sanitizers from two threads.

Execution plan (split to fit the 500-line budget), following `docs/research/ffi-native-ui.md` Part 4:

- T10.1 Handles and frame API in `scull-ffi`: opaque `tt_term` and `tt_frame` handles, `tt_status` codes, ABI version check at create, `struct_size` first in every struct, `catch_unwind` around every export with per-terminal poisoning (a poisoned handle answers `TT_POISONED` until freed), create/feed/resize/free, and `tt_frame_update` exposing T9's flat cells, text runs, damage and cursor as `repr(C)` views valid until the next update.
- T10.2 Threads and events: own the `scull-pty` reader thread per terminal, one wakeup callback (coalesced, never called under a lock), a polled event queue (replies written back, title, bell, clipboard requests, child exit), 2026 holds driven by `sync_held`/`sync_deadline`, and the resize contract from T11 (pause reads, one resize, resume).
- T10.3 Generated bindings: `include/scull.h` with cbindgen and C# bindings with csbindgen, committed; a `just` recipe regenerates them and CI fails on diff. A C test program (`create`, `feed`, `update`, `free` from two threads) built and run under ASan, UBSan and TSan where the platform supports them.
- Verify: Rust tests for every status path and for panic poisoning; `just check` exit 0; the C program under sanitizers.
