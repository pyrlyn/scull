# T8. PTY and I/O thread

`scull-pty`: a reader thread per terminal on top of a PTY crate, bounded work per lock hold, `try_lock` with back-pressure, replies written from the same thread, child exit through the same loop. ConPTY on Windows. Done when a flooding child cannot starve a frame read and a shell runs on both platforms.

Execution plan:
1. Re-check `portable-pty` maintenance (research.md §6) against alternatives on crates.io; pick one and record why.
2. `crates/scull-pty`: spawn a child on a PTY (ConPTY on Windows), a reader thread per terminal that hands bounded chunks to a caller-supplied sink, `try_lock`-style back-pressure through a trait so the crate stays independent of scull-term, replies written from the same thread through a capped queue, resize, and child exit delivered through the same loop.
3. Tests: a flooding child (`yes`) cannot starve a concurrent reader of the shared state; a shell runs `echo` and exits with its status; resize reaches the child; on Windows the same tests through ConPTY.
4. Verify with `just check`.
