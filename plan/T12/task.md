# T12. Input encoding

`scull-input`: pure encoders for legacy keys, the kitty keyboard protocol with a flag stack per screen, win32-input-mode, all mouse modes including SGR-pixel, bracketed paste with end-marker stripping, and focus events. Done when the table-driven tests match xterm and the kitty specification.

Execution plan:
1. Look for reusable encoders in the workspace and upstream (Alacritty, WezTerm `termwiz` input encoding, Windows Terminal win32-input-mode spec); port with notices or reimplement from the kitty keyboard protocol and xterm ctlseqs specifications.
2. `crates/scull-input`: a platform-neutral key struct (physical key, modifiers, consumed modifiers, text, unshifted code point, composing flag); pure encoders for legacy xterm keys (cursor/keypad application modes, modifyOtherKeys), the kitty keyboard protocol with a per-screen flag stack (push/pop/query, capped depth), win32-input-mode, mouse in X10, normal, button, any-event, UTF-8, SGR, urxvt and SGR-pixel modes, bracketed paste with end-marker stripping, and focus events.
3. Table-driven tests against xterm and the kitty specification; a proptest that no encoder panics.
4. Verify with `just check`. Split at claim: T12.1 keys (legacy + kitty + win32); T12.2 mouse, paste and focus.
