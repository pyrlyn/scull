> [!NOTE]
> This project is under active development. We need "testing volunteers": try it and report what breaks.

# Scull

A terminal emulator with a Rust core and a native interface on each platform: SwiftUI on macOS, WinUI on Windows. The core parses, keeps state and encodes input; the platform draws text with its own text stack and handles windows, input methods and accessibility.

Status: an empty Cargo workspace (`just check` builds, lints and tests it). See `plan.md` for what comes next.

- [research.md](research.md) — how kitty, WezTerm, Alacritty, foot, Contour and Warp work, and what Scull takes from each.
- [plan.md](plan.md) — active tasks.
- [roadmap.md](roadmap.md) — approved later work.
