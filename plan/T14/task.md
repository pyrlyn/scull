# T14. macOS app: first light

A SwiftUI shell with an `NSView` surface that draws frames with CoreText, sends key and mouse events, and resizes. Done when a shell is usable in one window.

Execution plan (split to fit the budget):

- T14.1 Input through the C ABI: store the mouse tracking modes (1000/1002/1003, 1006 SGR, 1016 pixels), focus reporting (1004), bracketed paste (2004) and the kitty keyboard flags stack (CSI > u, < u, = u, ? u) in `scull-term`, reusing `scull-input`'s types rather than copying them; export `tt_term_key`, `tt_term_text`, `tt_term_mouse`, `tt_term_paste`, `tt_term_focus` and `tt_term_scroll_display` that encode with `scull-input` against the current modes and write to the PTY; regenerate the header and C# bindings, bump the ABI minor.
- T14.2 macOS app in `macos/`: a Swift package with the SwiftUI app and an `NSView` surface, a module map over `scull.h`, and a `just macos` recipe that builds `scull-ffi` as a static library for `aarch64-apple-darwin` and then the app bundle. The view draws the frame's text runs with CoreText (one `CTLine` per run, backgrounds, cursor, wide cells), maps `NSEvent` keys, text input and mouse to the T14.1 exports, sends pixel sizes on resize, and turns the wakeup callback into a main-queue redraw. Minimum macOS 26.
- Verify: Rust tests for every new export and mode; `just check` and `just c-abi-test` exit 0; `just macos` builds; the app runs the user's shell in one window, and a screenshot after typing `ls` shows the output.
