# T13. Selection, search, links, shell integration

Selection and scrollback search in the core. OSC 8 links as per-row ranges. OSC 52 as an event the UI may deny. OSC 7 and OSC 133. Title, bell and notification events. Done when each has a golden test and an event in the queue.

Execution plan:
1. Reuse `Row::set_link` / `LinkSpan` in scull-grid and the coalesced bell flag (`take_bell`). No selection or OSC 7/8/52/133 handler exists under apps, packages or tools. `base64` is already a workspace dependency (scull-image); OSC 52 uses it for a one-shot decode.
2. T13.1, landed: a capped polled event queue in scull-term for OSC 0/1/2 titles, BEL, OSC 7, OSC 133, OSC 52 (the host may deny or answer a read) and OSC 8 stamped onto printed cells. The host reads a title or directory from the event; `link_uri` resolves an id later. Golden fixtures in `tests/fixtures/shell.txt`.
3. T13.2 (core selection and search), T13.3 (events over the C ABI) and T13.4 (macOS selection, copy and find UI) landed; see their entries.
