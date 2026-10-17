> [!NOTE]
> This project is under active development. We need "testing volunteers": try it and report what breaks.

# Scull

A terminal emulator with a Rust core and a native interface on each platform: SwiftUI on macOS, WinUI on Windows. The core parses, keeps state and encodes input; the platform draws text with its own text stack and handles windows, input methods and accessibility.

Status: early development, no release yet. The Rust core lives in [`crates/`](crates/) (parser,
grid, PTY, input, Unicode, image, config and the C ABI in `scull-ffi`); the SwiftUI app is in
[`macos/`](macos/) and the WinUI app in [`windows/`](windows/).

```bash
just check         # format, lint and test the Rust workspace
just macos         # build the macOS app
just windows-app   # build the Windows app
```

- [docs/](docs/README.md) — benchmarks, research notes and the config schema.
- [research.md](research.md) — how kitty, WezTerm, Alacritty, foot, Contour and Warp work, and what Scull takes from each.
- [plan.md](plan.md) — active tasks.
- [roadmap.md](roadmap.md) — approved later work.

## License

You can use this project under **any** of the following licenses, at your choice:

1. [GNU GPLv3](LICENSE): free for open source applications on any platform, including embedded systems.
2. [Royalty-free License](LICENSE-ROYALTY-FREE.md): free for proprietary desktop, mobile, and web applications, as long as you disclose that your application uses this project. Embedded systems are not covered.
3. [Commercial license](PRICING.md): for proprietary applications, including embedded systems, without the attribution requirement.

<!-- license-sync:start -->
Commercial use not covered by the GPLv3 or the Royalty-free License requires a separate paid
license — see [PRICING.md](PRICING.md).
<!-- license-sync:end -->
