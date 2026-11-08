# T4. Parser

`scull-parser`: a bulk scanner that delivers text up to the next control byte as one run, in front of a table state machine for CSI, OSC, DCS and APC, emitting typed actions. Payloads are dispatched in place with hard length caps. Done when the fuzz target runs clean and throughput on plain text beats `vte` in the T3 benchmark.

Execution plan:
1. Pick the state machine: `vtparse` behind our scanner (research.md §6) or our own table after Paul Williams' DEC parser; record why in the commit.
2. `crates/scull-parser`: bulk scanner (printable UTF-8 runs up to the next control byte as one `&str`, split UTF-8 kept across `feed` calls), CSI with colon sub-parameters, ESC, OSC, DCS and APC delivered in place to a handler trait; hard caps on parameters, intermediates and payload lengths, each with a test.
3. `fuzz/` workspace (cargo-fuzz, nightly) with a parser target; run it clean locally.
4. A divan bench in `crates/scull-parser/benches` against `vte` on plain text and mixed SGR output.
5. Verify with `just check`. Over the 500-line budget, stop at a coherent boundary and split the rest into T4.x.
