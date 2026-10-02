# Ideas

Not approved. Nothing here moves to `roadmap.md` or `plan.md` without the creator.

- Cold scrollback tier: old rows stored as flat text plus run-length attributes, so long history costs far less than full cells and reflow becomes an index rebuild.
- Text sizing protocol (OSC 66): wide characters and scaled text as one multicell representation in the grid.
- Command blocks built on OSC 133 marks: jump between prompts, select or copy one command's output.
- Packaging and signed releases: XCFramework and notarised app on macOS, MSIX on Windows.
- Linux front-end (GTK) over the same C ABI.
- Bidirectional text in the core, or left to the platform text stacks.
- Session persistence and a detachable server process.
- A fallback 2D renderer on Windows for remote desktop and machines without a GPU.
- Publishing the core as a standalone library for other terminals to embed.
