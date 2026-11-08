# T24. Reconcile harness eager-wrap with xterm's deferred DECAWM

`crates/scull-harness` wrapped eagerly at the last column while xterm DECAWM defers the wrap until the next printable. The golden fixtures encoded that eager wrap and would diverge from `scull-term`. Done means the wrap semantics the fixtures assert are the ones the real core has, or the divergence is documented as intentional for the stub.

Execution plan:
1. Read `scull-term` DECAWM (`pending_wrap`) and the harness stub. Search the workspace for an existing wrap helper before writing one.
2. Make the stub defer the wrap the way the core does, and update the golden fixture so it fails under an eager wrap.
3. Add a test named for the xterm rule: CR stays on the same line, and only the next printable wraps.
4. Leave `scull-term` unchanged. Verify with `mise exec` fmt, clippy `-D warnings`, and nextest for `scull-harness`.
