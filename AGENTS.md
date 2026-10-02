# Scull — instructions for agents

**What this is.** A terminal emulator: Rust core, native UI per platform (SwiftUI on macOS, WinUI on Windows), joined by a C ABI. Rust computes, the platform renders.

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too; on conflict, ask the creator.

## Read first

- `research.md` — the architecture report and the reasons behind every choice.
- `docs/research/` — per-terminal findings with `path:line` sources.
- `plan.md` — active tasks. T1 (open decisions) blocks all other work.

## Hard rules

- **No UI types in the core.** Core crates speak cells, rows, columns and plain integers. Pixel metrics, colours of the toolkit, key types of the toolkit and windows stay on the platform side.
- **The UI never holds a core lock.** It reads frames through the snapshot API only.
- **The core never calls the UI while holding a lock.** The only callback is the wakeup; everything else is a polled event.
- **No panic crosses the C ABI.** Every export is wrapped; a panic poisons one terminal.
- **Everything from the PTY is untrusted input.** Every payload, table and queue it can grow has a hard cap and a test.
- **Licences.** The project is `GPL-3.0-or-later`. kitty's file headers say "GPL3" with no "or later" wording, so any file ported from kitty is marked `GPL-3.0-only` and kept apart from our own files; prefer reimplementing from kitty's protocol specifications. Code may be ported from MIT, Apache-2.0 and GPL-3 sources (Alacritty, WezTerm, foot, Contour, Ghostty, Windows Terminal, kitty) with their notices kept and the origin named in the commit. Warp's core is AGPL-3.0: study it, never port it. Take Alacritty-derived code from upstream Alacritty, not from Warp's copies.
- **Claims of speed need a number.** "Faster" or "better" goes into a doc or commit only with a benchmark from the harness.

## Decisions

Settled by the creator; reasons are in `research.md` §8.

- The structural reference named "Wrapp" in the brief is Warp.
- **Windows host:** C# only (WinUI 3), shell and surface alike. No C++/WinRT component. Direct3D, DirectWrite, TSF and UIA are reached from C# through an interop library that T17 picks.
- **VT core:** our own Rust state and grid on leaf crates. libghostty-vt is not embedded.
- **PTY:** inside the core.
- **Minimum macOS:** 26. No fallbacks for older systems.
- **Split panes on Windows:** one swap chain per window; panes are viewports.
- **Project licence:** `GPL-3.0-or-later`. The creator sells subscription services around the terminal; the client itself stays GPL.
