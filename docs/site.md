---
title: Scull
tagline: A terminal emulator with a Rust core and a native interface on each platform — SwiftUI on macOS, WinUI on Windows.
repo: https://github.com/pyrlyn/scull
install: "git clone https://github.com/pyrlyn/scull && cd scull && just check"
version: "0.0.0"
status: "Early development · no release yet"
accent: "#7FD1E8"
order: 11
---

<!-- Website copy for the pyrlyn project site, copied to pyrlyn/landing as
content/projects/scull.md; front matter follows CONTENT_CONTRACT.md in that repository.
Sources (checked 2026-10-14): README.md, Justfile and research.md; version from the workspace Cargo.toml (no release yet, hence status). The accent is a placeholder: the repository has no brand colour yet. -->

## Overview

Scull is a terminal emulator with a Rust core and a native interface on each platform. The core
parses, keeps state and encodes input; the platform draws text with its own text stack and
handles windows, input methods and accessibility.

Scull is in early development with no release: the Rust core crates, a SwiftUI app in `macos/`
and a WinUI app in `windows/` build from the repository.

## Features

- **Rust core.** Parser, grid, PTY, input, Unicode, image and config crates.
- **Native UI per platform.** SwiftUI on macOS and WinUI on Windows, over a C ABI (`scull-ffi`).
- **Platform text stacks.** Each platform draws text, handles input methods and accessibility itself.
- **Config schema.** A JSON Schema for the config file is generated from the code.

## Install

There is no release yet. Build from the repository:

```bash
git clone https://github.com/pyrlyn/scull && cd scull && just check
```

## Usage examples

Build the native apps:

```bash
just macos
just windows-app
```

## Links

- Repository: <https://github.com/pyrlyn/scull>
- Research: <https://github.com/pyrlyn/scull/blob/main/research.md>
- Documentation: <https://github.com/pyrlyn/scull/tree/main/docs>
