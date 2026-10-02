# Warp — source-level structural analysis

- Repository: https://github.com/warpdotdev/warp, branch `master`
- Analysed commit: `38b2c55e4ed2cd9a19fc66e13e323d976446ca40` ("Write the gh hosts file where gh reads it on Windows (#16238)"), shallow clone taken 2026-10-02
- All `path:line` citations are relative to the repo root at that commit.
- Method: read-only reading of the source. Nothing was built or run. Statements labelled **Assessment** are my evaluation, not facts from the source. Anything not read is marked **unverified**.

## 0. Licence facts (read first)

| Fact | Source |
| --- | --- |
| Workspace licence is `AGPL-3.0-only` | `Cargo.toml:27` |
| `warpui` and `warpui_core` (the UI framework) are **MIT**, not AGPL | `crates/warpui/Cargo.toml:7`, `crates/warpui_core/Cargo.toml:7`, `README.md:52-56`, files `LICENSE-MIT`, `LICENSE-AGPL` at repo root |
| `warp_terminal` (PTY, parser driver, grid) inherits the workspace licence, i.e. AGPL | `crates/warp_terminal/Cargo.toml:6` (`license.workspace = true`) |
| Large parts of `warp_terminal` are marked "adapted from alacritty_terminal under the Apache license" with a bundled `LICENSE-ALACRITTY` | file headers of `local_tty/mod.rs`, `local_tty/event_loop.rs`, `local_tty/unix.rs`, `model/mode.rs`, `model/grid/cell.rs`, `model/grid/storage.rs`, `model/grid/grid_storage.rs`, `model/grid/resize.rs`; `app/src/terminal/ref_tests/mod.rs:1-2` |
| The VT parser is a Warp fork of `vte`: upstream `alacritty/vte` tag `v0.13.0` plus exactly one commit "Copied APC support logic (#1)" touching `src/definitions.rs`, `src/lib.rs`, `src/table.rs` | `Cargo.toml:348` (rev `4b399c87b63ba88f45709edaa6383fc519f6c900`), `Cargo.lock:15186-15191`; GitHub compare API `alacritty:vte:v0.13.0...4b399c87` checked 2026-10-02 (ahead 1, behind 0) |

Consequence for us (**Assessment**): we may study everything. Code may only be reused from (a) the MIT crates `warpui`/`warpui_core`, or (b) the original Apache-2.0 upstream (`alacritty_terminal`, `vte`) — never from Warp's AGPL-modified copies. Warp-original pieces of the core (flat scrollback storage, block model, DCS/OSC shell hooks, sync-output buffering, hyperlink registry, image map, kitty keyboard encoder) are AGPL: structure only, clean-room reimplementation. See section C.4 for the list of places where "the only practical reuse would be copying".

## A. Workspace structure

### A.1 Crates and layering

- Workspace = `app/` plus `crates/*`; `AGENTS.md:121-125` says "60+ member crates". Toolchain Rust 1.92.0, edition 2024 (`rust-toolchain.toml`, `rustfmt.toml`).
- Two front-ends share one core: the GUI (`app/`, on `warpui`/`warpui_core`, GPU rendered) and a headless TUI (`crates/warp_tui`, `TuiElement` under `crates/warpui_core/src/elements/tui`, feature `tui`) — `AGENTS.md:69-79`; `crates/warp_tui/Cargo.toml:73-90` depends on `warp`, `warp_terminal`, `warpui`, `warpui_core[tui]`; `warpui_core` pulls `ratatui 0.30` behind the `tui` feature (`crates/warpui_core/Cargo.toml:11,66`).
- Layering as declared by manifests:
  - `warpui_core` (MIT): entity/handle app core, elements, scene, text-layout abstractions, platform traits. Depends on `warp_errors`, `warp_util`, `sum_tree`, no terminal code (`crates/warpui_core/Cargo.toml:25-89`).
  - `warpui` (MIT): platform back-ends (`platform/{mac,linux,windows,wasm,headless}`, `windowing/winit`, `rendering/{atlas,glyph_cache,wgpu}`), re-exports all of `warpui_core` (`crates/warpui/src/lib.rs:1-8`).
  - `warp_core`: feature flags, channel, settings, safe logging; depends on `warpui_core` (`crates/warp_core/Cargo.toml:57`).
  - `warp_terminal`: PTY + event loop + ANSI processor + grid. Depends on `warp_core` and `warpui_core` (`crates/warp_terminal/Cargo.toml:67,71`), `vte`, `mio`, `nix`/`signal-hook` (unix), `windows` (ConPTY). Modules: `crates/warp_terminal/src/lib.rs:1-18`, `src/model/mod.rs:1-35`, `src/model/grid/mod.rs:1-40`.
  - `app` (crate `warp`): everything else, including `TerminalModel`, blocks, alt screen, renderer, views. Depends on `warp_terminal` (`app/Cargo.toml:247`) and re-exports its model modules (`app/src/terminal/model/mod.rs:1-41`).

### A.2 How isolated is the terminal model?

Not fully. Two facts matter:

1. The low-level grid lives in `warp_terminal`, but the top-level model that implements `ansi::Handler` for a session — `TerminalModel` — lives in the **app** crate (`app/src/terminal/model/terminal_model.rs:376-534`), next to `BlockList`, `Block`, `AltScreen`. Its imports include `warpui::AppContext`, `crate::ai::*`, session-sharing protocol types (`terminal_model.rs:1-60`).
2. `warp_terminal` itself depends on the UI framework's core. Observed imports (grep over `crates/warp_terminal/src`): `warpui_core::color::ColorU`, `units::{Pixels, Lines}`, `keymap::Keystroke`, `platform::OperatingSystem`, `platform::keyboard::KeyCode`, `event::ModifiersState`, `image_cache::*`, `assets::asset_cache::Asset`, `text::SelectionType`, and `AppContext`/`Entity`/`SingletonEntity`/`ModelContext` in `local_tty/{unix,spawner,terminal_attributes,windows/mod}.rs`. `SizeInfo` (the model's size type) is built on `warpui_core::units::Pixels` and pathfinder `Vector2F` (`crates/warp_terminal/src/runtime.rs:21-60`).

**Assessment:** Warp's "core" is a crate boundary for build/perf reasons (`warp_terminal.opt-level = 3` in the dev profile, `Cargo.toml:450`), not a UI-agnostic library. It is not a model for an FFI-clean core; the alacritty split (`alacritty_terminal` with zero UI deps) is cleaner.

### A.3 Threading model

- One OS thread per PTY, named "PTY reader", running a `mio` poll loop: `crates/warp_terminal/src/local_tty/event_loop.rs:328-498`. Tokens CHANNEL/PTY/SIGNALS (`:32-34`). Messages into the loop: `Input`, `Shutdown`, `Resize`, `ChildExited` (`:168-179`).
- The loop owns the parser (`State { write_list, writing, parser: ansi::Processor }`, `:68-72`) and shares the model as `Arc<parking_lot::FairMutex<M>>` where `M: ActiveTerminal` = `ansi::Handler + Send + exit()` (`:40-56`). The app implements `ActiveTerminal` for `TerminalModel` (`app/src/terminal/local_tty/mod.rs:25-29`).
- `pty_read` (`event_loop.rs:199-285`): read buffer 256 KiB (`READ_BUFFER_SIZE = 0x4_0000`, `:26`), `try_lock` first and only block on the lock when the buffer is full; at most 64 KiB parsed per lock hold (`MAX_LOCKED_READ = 0x1_0000`, `:30`); parse in place under the lock (`parser.parse_bytes(terminal, buf, &mut responses)`); terminal responses (DSR etc.) are queued to the PTY write list; `FairMutexGuard::bump` yields to waiting UI; a wakeup is sent only if bytes were actually applied (not just buffered by synchronized output) (`:280-282`).
- Wakeups/events leave the thread through channels held by `ChannelEventListener { wakeups_tx: async_channel::Sender<()>, terminal events, app events (Box<dyn Any+Send>), pty_reads_tx: async_broadcast }` (`crates/warp_terminal/src/event_listener.rs:17-98`). Channels are created in `app/src/terminal/local_tty/terminal_manager.rs:331-342`; the model is wrapped at `:386`; the loop is started at `:885-897`.
- UI side: the view consumes `wakeups_rx` throttled to 60/s (`app/src/terminal/view.rs:621-623`, `:3801-3805`) and calls `handle_terminal_wakeup` (`:10192-10237`), which locks the model several times for bookkeeping and ends with `ctx.notify()` (re-render). The TUI does the same (`crates/warp_tui/src/terminal_session_view.rs:2286-2292`). Model events are consumed on the UI thread by `ModelEventDispatcher` via `ctx.spawn_stream_local` and re-emitted as framework events (`app/src/terminal/model_events.rs:41-79`, `ModelEvent` `:380`).
- Input direction: views emit events that collapse to a narrow `PtyIntent` enum (`CtrlD`, `Interrupt`, `ShutdownPty`, `WriteBytes`, `Resize`, `ExecuteCommand`, ...) via `trait PtyIntentEvent`; `trait TerminalSurface: Entity` is generic over GUI/TUI surfaces (`app/src/terminal/writeable_pty/terminal_surface.rs:18-57`). A `PtyController` entity forwards bytes to the loop's `mio_channel::Sender<Message>`.
- Locking discipline is a documented hazard: `AGENTS.md:177-181` ("Terminal Model Locking": double `model.lock()` deadlocks/freezes the UI; pass locked refs down; keep scope short). `FairMutex` is not re-entrant; the code base has dozens of lock sites in elements alone (`app/src/terminal/block_list_element.rs` 19 sites, `alt_screen_element.rs` 11 sites — grep `model.lock()`).

### A.4 How the grid reaches rendering

There is **no snapshot and no damage list**. Elements hold the same `Arc<FairMutex<TerminalModel>>` and lock it inside `paint`:

- `AltScreenElement::paint` takes `self.model.lock()` and passes `&GridHandler` straight to `grid_renderer::render_grid` for rows `[start_row, end_row)` (`app/src/terminal/alt_screen/alt_screen_element.rs:680-760`).
- Blocks: `impl BlockGridRenderer for BlockGrid` computes visible rows from clip bounds and calls the same `render_grid` (`app/src/terminal/blockgrid_renderer.rs:90-166`).
- `render_grid` iterates every visible row and every column (`for col in 0..grid.columns()`, `app/src/terminal/grid_renderer.rs:624-643`) on each paint.
- `dirty_cells_range` / `find_dirty_rows_range` exist in the grid but are consumed only by secret scanning, block filtering and find (`crates/warp_terminal/src/model/grid/{secrets.rs:163,filtering.rs:446}`, `app/src/terminal/find/model.rs:476-489`), never by the renderer (grep over `app/src` and `crates`).

So the PTY thread is blocked from parsing for the duration of every paint of that terminal, and vice versa; fairness comes from `FairMutex` + the 64 KiB cap.

### A.5 Repo docs and code-quality conventions

- Agent/dev docs: `AGENTS.md` (252 lines; no root `CLAUDE.md`), `.agents/{skills,specs}`, `.claude/{settings.json,skills}`, `specs/` with 291 `APP-xxxx` entries, `crates/warp_terminal/src/model/ESCAPE_SEQUENCES.md`.
- Architecture statement: Entity–Component–Handle framework; global `App` owns views/models, `ViewHandle<T>`/`ModelHandle<T>`, Flutter-style `Element`s (`AGENTS.md:83-93`).
- Lints: `.clippy.toml` — disallowed `dbg!`; disallowed types `std::time::Instant` (wasm; use `instant::Instant`), `std::process::Command` and `async_process::Command` (use the `command` crate); disallowed methods `async_channel::Sender::send_blocking`, `LineEnding::from_current_platform`. Clippy runs with `-D warnings` over the workspace and again for `-p warp` (`script/presubmit`). No `[workspace.lints]` table in root `Cargo.toml` (grep empty).
- Presubmit (`script/presubmit`): `./script/format --check`, `./script/check_no_inline_test_modules`, clippy, clang-format for C/ObjC in `crates/warpui/src` and `app/src`, `wgslfmt --check`, ShellCheck, PSScriptAnalyzer.
- Style rules (`AGENTS.md:127-175, 250-252`): context param last and named `ctx`; remove unused params; inline format args; comments explain why; `max_width 100`; exhaustive matches (avoid `_`).
- Testing (`AGENTS.md:183-193`): `cargo nextest`; unit tests in sibling `*_tests.rs` files included by `#[cfg(test)] #[path = ...] mod tests;` (enforced by the presubmit check). 943 test files in `app` + `crates`, 29 in `warp_terminal` (`find -name '*_tests.rs' -o -name mod_test.rs`). Nextest config: 30 s slow timeout, integration tests count double, CI retries (`.config/nextest.toml`).
- Terminal conformance tests: alacritty-style **ref tests** (recorded byte stream + expected grid JSON) in `app/src/terminal/ref_tests/` (harness "adapted from alacritty_terminal", `mod.rs:1-2`; 40 data dirs incl. `vttest_*`, `tmux_htop`, `vim_*`, `zerowidth`, `mod.rs:47-80`). Note they live in the app crate because `TerminalModel` does.
- Error handling: `thiserror` enums at boundaries (e.g. `PtySpawnError`, `crates/warp_terminal/src/local_tty/windows/mod.rs:134-162`; `crates/warp_terminal/src/event.rs`), `anyhow` inside; non-fatal invariants reported with `report_error!`/`report_if_error!` (`crates/warp_errors/src/lib.rs:56,188`) and `safe_assert!`, `safe_*` logging macros (`crates/warp_core/src/{assertions.rs:8,safe_log.rs:7-91}`). Example of the pattern in the renderer: an out-of-bounds row is reported once per run and skipped rather than panicking (`app/src/terminal/grid_renderer.rs:627-641`).
- Feature flags: runtime `FeatureFlag::X.is_enabled()` preferred over `cfg` (`AGENTS.md:216-248`); the terminal core consults them directly (e.g. `OscHyperlinks`, `KittyImages`, `SequentialStorage`, `BoxDrawingGlyphs`).
- Persistence via Diesel + SQLite (`AGENTS.md:207-210`).

## B. Pipeline: bytes to pixels

### B.1 PTY I/O

- Abstractions (alacritty-derived): `trait EventedReadWrite` (mio register/reader/writer, `crates/warp_terminal/src/local_tty/mod.rs:39-51`), `trait EventedPty` (`child_event_token`, `next_child_event`, `on_resize(&SizeInfo)`, `kill`, `:65-81`), `PtyOptions` (`:84-107`).
- Unix (`local_tty/unix.rs`): `nix::pty::openpty` (`:47-52`); child setup in `spawn_command_in_pty` (`:463-588`): IUTF8, `pre_exec` resets signals, dup2 to stdio, `setsid`, `TIOCSCTTY`, optional close of fds ≥ 3; non-blocking master, SIGCHLD via `signal-hook` (`:592-632`); resize = `TIOCSWINSZ` (`:717-723`).
- Warp-specific: a separate **terminal server** child process, started early, forks shells so the GUI process's fds/threads do not leak; talks over Unix sockets on fds 3/4 (`local_tty/server/mod.rs:1-69`); `PtySpawner` singleton tries the server and falls back to direct spawn (`local_tty/spawner.rs:121-264`).
- Windows (`local_tty/windows/mod.rs:164-273`): ConPTY loaded dynamically from a **bundled `conpty.dll`** (`conpty_api.rs:53-90`, `LoadLibraryW("conpty.dll")` + `GetProcAddress` for Create/Resize/Close/ShowHide/Release), anonymous duplex pipes (`pipes.rs:82`), `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_BREAKAWAY_FROM_JOB`, `mio::windows::NamedPipe`, a child-exit watcher that posts into the loop channel; resize `:511-517`; close `:431-453`.
- ConPTY-specific workaround: OSC 9279 "reset grid" so the shell can resynchronise the grid, plus `PerformResetGridChecks` (`model/ansi/mod.rs:44-81`, `model/grid/grid_handler.rs:381-393`).

### B.2 Parsing

- `Processor { state, parser: VteParser }` → `Performer` (implements `vte::Perform`) → `trait Handler` (`crates/warp_terminal/src/model/ansi/mod.rs:1-11, 369-372`). `Handler` has ~110 methods: the alacritty set plus Warp hooks (`command_finished`, `precmd…`, `preexec`, `bootstrapped`, `init_shell`, `prompt_marker`, `set_hyperlink`, image handlers, `on_finish_byte_processing`, keyboard-enhancement flags) (`model/ansi/handler.rs:25-413`).
- Divergence from upstream vte: only APC callbacks (`apc_start/apc_put/apc_end`), used for kitty graphics (`'G'`, gated by `FeatureFlag::KittyImages`) (`model/ansi/mod.rs:1600-1632`; fork facts in section 0).
- Synchronized output (?2026) is implemented in the processor, not the grid: bytes between `CSI ?2026h` and `l` are buffered (max 2 MiB, timeout 150 ms) and replayed (`model/ansi/mod.rs:62,69,257-328,444-514`); DECRQM answers for 2026 (`:1438-1463`). The event loop uses the remaining timeout as its poll timeout.
- OSC handled: 0/2, 4, 7, 8 (flagged), 9, 10/11/12, 50, 52, 104, 110–112, 133, 777, 1337 (iTerm files), and Warp-private 9277 (in-band generator output), 9278 (hooks), 9279 (reset grid), 9280 (completions) (`model/ansi/mod.rs:825-1176`).
- DCS: only Warp hooks, final byte `d` (hex-encoded JSON) / `f` (raw JSON) (`model/ansi/mod.rs:762-790`, `model/ansi/dcs_hooks.rs:17-90`). **No sixel**: `grep -rni sixel app crates` returns nothing.
- CSI: standard set, DECSCUSR, XTVERSION, `CSI t` 14/18/22/23, kitty keyboard `CSI = > < ? u` — the last is compiled out on Windows (`cfg!(not(windows))`) because ConPTY cannot forward it (`model/ansi/mod.rs:1288-1541`).
- Modes recognised: ?1, 3, 6, 7, 12, 25, 47, 1000, 1002, 1003, 1004, 1005, 1006, 1007, 1042, 1049, 2004, 2026; ANSI 4, 20 (`model/ansi/control_sequence_parameters.rs:49-162`).

### B.3 Terminal state

- `TermMode: u32` bitflags incl. kitty keyboard bits (`model/mode.rs:8-89`).
- Per-grid `State` (cell size, mode, tab stops, cursor style, active charset, scroll region, alt-screen flag, dirty range, keyboard-mode stack) (`model/grid/ansi_handler.rs:51-105`).
- Cursor = `{ point, template: Cell, charsets: [StandardCharset; 4], input_needs_wrap }` (`model/grid/grid_storage.rs:31-50`); charsets are ASCII and DEC special graphics only (`model/ansi/mod.rs:1544-1598`).
- SGR parsing `attrs_from_sgr_parameters` (`control_sequence_parameters.rs:517`); colours `Named/Spec/Indexed`.

### B.4 Grid and cells

- `Cell { c: char, fg, bg, flags: u16, extra: Option<Box<CellExtra>> }`, documented as exactly 24 bytes; `CellExtra` holds zero-width string, end-of-prompt marker, hyperlink id (`model/grid/cell.rs:33-152`).
- `Row { inner: Vec<Cell>, occ }` (`row.rs:15-25`); `Storage` ring buffer (`storage.rs:90-116`); `GridStorage` (`grid_storage.rs:83-122`).
- `GridHandler` = active-region `GridStorage` (scrollback forced to 0) + `FlatStorage` scrollback + hyperlink registry + images + secrets + filter state (`model/grid/grid_handler.rs:416-520`).
- Alt screen = a separate `GridHandler` with no scrollback inside `AltScreen` (`app/src/terminal/model/alt_screen.rs:42-93`; created with `max_scroll_limit` 0 at `terminal_model.rs:1065`); `TerminalModel` routes handler calls to block list or alt screen with a `delegate!` macro (`terminal_model.rs:2597+`).

### B.5 Scrollback and blocks

- `FlatStorage` (Warp-original): UTF-8 content in 1 KiB chunks keyed by byte offset (`flat_storage/content.rs:35-48,170-186`), a row index of content offsets + grapheme sizing (`index.rs:1-67`), and run-length attribute maps for fg, bg+style, hyperlink id (`attribute_map.rs:16-23`, `style.rs`), with `max_rows` truncation (`flat_storage/mod.rs:1-102`). Rows scrolled out of the active grid are converted into this form.
- Blocks: `BlockList { blocks: Vec<Block>, block_heights: SumTree<BlockHeightItem>, ..., max_grid_size_limit }` (`app/src/terminal/model/blocks.rs:268-330`); `Block { header_grid, rprompt_grid: BlockGrid, output_grid: BlockGrid, state, exit_code, timestamps, ... }` (`model/block.rs:297-345`); `BlockGrid` wraps a `GridHandler` with started/finished state and memoised queries (`crates/warp_terminal/src/model/blockgrid.rs:30-110`). On finish a grid is truncated to the cursor and may be compacted entirely into flat storage (`grid_handler.rs:1838-1860`).
- Scrollback limit is **per block**: setting `terminal.maximum_grid_size`, default 50 000 rows (`app/src/terminal/settings.rs:153-162`; plumbed at `blocks.rs:686,898`).
- Block boundaries come from shell hooks (B.9); output arriving between blocks goes to an `EarlyOutput` model (`blocks.rs:560-570`).

### B.6 Unicode

- Width: `unicode_width::UnicodeWidthChar` per scalar (`model/grid/ansi_handler.rs:20,195`); workspace pins `unicode-width = "0.1.12"` (`Cargo.toml:344`).
- Zero-width scalars are appended to the previous cell's `CellExtra` string (cap `MAX_GRAPHEME_BYTES = 256`); the whole cell string is then re-measured with `UnicodeWidthStr`, and if it became width 2 (e.g. U+2601 U+FE0F) the cell is rewritten as wide — except for zsh (`ansi_handler.rs:181-310`, esp. `:243-269`).
- No grapheme-cluster segmentation at input time: `unicode-segmentation` is only a dev-dependency of `warp_terminal` (`crates/warp_terminal/Cargo.toml:120`). ZWJ emoji sequences therefore depend on per-scalar widths. **Assessment:** this is the weakest part of the core relative to current practice (mode 2027 grapheme clustering is absent from the mode table in B.2).
- Rendering of multi-scalar cells shapes the cell string as a unit (`grid_renderer.rs:1762-1774`, `grid_renderer/cell_glyph_cache.rs:35-60`).

### B.7 Resize / reflow

- `GridHandler::resize` (`model/grid/resize.rs:15-179`): alt screen → no reflow; otherwise push all active rows into flat storage, convert cursors to content offsets, `flat_storage.set_columns(n)` (reflow = rebuild the row index; text is not re-parsed), pop visible rows back, restore cursors, apply max rows, then rescan secrets and refilter. The alacritty-style row reflow remains for `GridStorage` (`grid_storage/resize.rs:16-45`).
- `MIN_ROWS = 1`, `MIN_COLUMNS = 2` (`runtime.rs:11-16`).

### B.8 Input encoding

- Keyboard: `KeystrokeWithDetails::to_escape_sequence(mode_provider)` in `crates/warp_terminal/src/model/escape_sequences.rs:235-300`, helpers for C0, fn keys with xterm modifier params, cursor keys (SS3 vs CSI by `APP_CURSOR`), meta prefix (`:304-677`). Mode access is abstracted by `trait ModeProvider { fn is_term_mode_set }` (`:223`).
- Kitty keyboard: `escape_sequences/kitty_keyboard_protocol.rs:1-70` implements flags 1 (disambiguate) and 8 (report all) with macOS Option-composition handling; mode stack in grid state. Disabled on Windows (B.2). Coverage of flags 2/4/16 is **unverified** (not read past line 70).
- Mouse: `MouseState { button, action, point, modifiers }` (`model/mouse.rs:1-57`) → `to_escape_sequence` (`escape_sequences.rs:329`); formats documented in `model/ESCAPE_SEQUENCES.md` (SGR `CSI < b;col;row M/m`). Elements decide whether the app or Warp gets the mouse (`alt_screen_element.rs:278-443`). Exact encodings for modes 1000/1005 were not read — **unverified**.
- Bracketed paste: constants `ESC[200~`/`ESC[201~` (`escape_sequences.rs:157-158`), applied in `app/src/terminal/writeable_pty/pty_controller.rs:400-402,832-836`.
- Focus events: `FOCUS_IN_OUT` mode set by ?1004 (`grid/ansi_handler.rs:1012,1079`); the view writes focus-in/out when the mode is set and a user setting allows it (`app/src/terminal/view.rs:8746-8759`).

### B.9 Hyperlinks and shell integration

- OSC 8: parsed into `Hyperlink` with caps `MAX_URI_BYTES = 2083`, `MAX_ID_BYTES = 256` (`control_sequence_parameters.rs:730-764`); interned in a per-grid `HyperlinkRegistry` capped at 4096 distinct entries, id stored in `CellExtra` / flat-storage attribute map (`model/grid/hyperlink_registry.rs:28-100`). Behind `FeatureFlag::OscHyperlinks` (`ansi/mod.rs:880`).
- Shell integration is mostly **private protocol**, not OSC 133: bundled bootstrap scripts for bash/zsh/fish/pwsh (`app/assets/bundled/bootstrap/`) emit `ESC P $ d <hex JSON> ESC \` (DCS) or `OSC 9278 ; d ; <hex> BEL` on WSL (`zsh_body.sh:17-45,79-90`), with hooks `CommandFinished`, `Precmd`, `Preexec`, `Bootstrapped`, `InitShell`, `InputBuffer`, `SSH`, `ExitShell`, ... as serde JSON `{"hook":..,"value":..}` (`model/ansi/dcs_hooks.rs:45-90`) and session-id validation (`dcs_hooks.rs:34`, `ansi/mod.rs:557`). OSC 133 prompt markers are also accepted (`ansi/mod.rs:1021`, `PromptMarker` `control_sequence_parameters.rs:622`).

### B.10 Images

- iTerm2 OSC 1337 `File`/`MultipartFile`/`FilePart`/`FileEnd` (`ansi/mod.rs:1065-1100`; `model/iterm_image.rs:8-59`).
- Kitty graphics over APC: actions store / store+display / display stored / query / delete; direct, file, temp-file and (unix) shared-memory transmission; zlib (`model/kitty.rs:20-130`).
- Placement: `ImageMap` keyed by absolute points that survive scrollback truncation via `num_lines_truncated` (`model/image_map.rs:12-70`); pixel data lives in the app (`TerminalModel.image_id_to_metadata`) and the UI image cache; painted before (z<0) or after text in a separate layer (`grid_renderer.rs:560-622`).
- Sixel: absent (B.2).

### B.11 Rendering

- Per paint, the element builds draw primitives into a `Scene` (layers of `rects`, `images`, `glyphs`, `icons`; `Glyph { glyph_key: {glyph_id, font_id, font_size}, position, fade, color }`) (`crates/warpui_core/src/scene.rs:18-98`). The scene is rebuilt by the presenter on invalidation (`crates/warpui_core/src/presenter.rs:318,333`; body not read — retained-vs-rebuilt details **unverified**).
- Two text paths (`grid_renderer.rs:320-462`):
  - without ligatures: per cell, map `(char, FontId)` → glyph through `CellGlyphCache` and `scene.draw_glyph` at `col * cell_width` (`:893-977,1702-1808`); the cache is created fresh in each paint (`alt_screen_element.rs:699`);
  - with ligatures: build one attributed string per row, shape it with the platform text-layout system, then snap each glyph back to its cell column (`:1459-1545`). The source itself calls this path "known to be less performant" (`:320-322`).
- Backgrounds are batched into runs; decorations and "native glyphs" are drawn after (`:540-547,1502-1519`). Box drawing, block elements, shades, quadrants and several Powerline/Nerd Font shapes are drawn as geometry rather than font glyphs (`:151-211,1915-1960`, `grid_renderer/box_drawing.rs`).
- Text layout / fonts are behind traits in the MIT core: `trait TextLayoutSystem: Send + Sync` and `trait FontDB` (`crates/warpui_core/src/platform/mod.rs:326-372`). Implementations: macOS = Core Text (`crates/warpui/src/platform/mac/text_layout.rs:10-28`) with font-kit rasterisation (`platform/mac/fonts.rs:24-42`); everything else = a Warp fork of `cosmic-text` + `swash` rasteriser, `fontdb`, DirectWrite font enumeration through font-kit/`dwrote` on Windows (`crates/warpui/Cargo.toml:157-166,212`, `windowing/winit/fonts.rs:21-23`, `fonts/swash_rasterizer.rs:1-60`, `fonts/windows.rs:26-40`).
- Glyph atlas: `GlyphCache<Texture>` keyed by `(GlyphKey, scale_factor, subpixel_alignment)`, 1024×1024 atlases, shelf-next-fit allocator, new texture when full, whole cache dropped when glyph config changes; no eviction (`crates/warpui/src/rendering/glyph_cache.rs:13-150`, `atlas/{manager.rs:43-67,allocator.rs:13-55}`).
- GPU back-ends: macOS uses a hand-written **Metal** renderer over AppKit (`platform/mac/rendering/metal/`, `shaders.metal`); all other platforms use winit + **wgpu** (`wgpu 30`, dx12/vulkan/gl/metal features); wgpu on macOS only behind `experimental-wgpu-renderer` (`crates/warpui/build.rs:13-22`, `Cargo.toml:375`, `platform/mac/rendering/renderer_manager.rs:6-41`). wgpu renderer = three instanced pipelines (rect, glyph, image), per-layer scissor, full clear every frame (`rendering/wgpu/renderer.rs:26-64`, `renderer/frame.rs:27-116`).
- Damage tracking: none at the terminal or GPU level (A.4; `LoadOp::Clear` in `frame.rs:108`). Redraw rate is bounded by the 60/s wakeup throttle.

## C. Core/UI separation and what transfers to "Rust core + SwiftUI/WinUI over FFI"

### C.1 The actual boundary in Warp

| Direction | Mechanism | Source |
| --- | --- | --- |
| PTY bytes → model | `trait ansi::Handler` called under `FairMutex` on the PTY thread | `event_loop.rs:199-285`, `ansi/handler.rs:25-413` |
| model → UI "something changed" | payload-free wakeup on an async channel, throttled by the receiver | `event_listener.rs:17-98`, `view.rs:3801-3805` |
| model → UI semantic events | typed `Event` enum over a channel, re-dispatched as entity events | `event.rs:152-167`, `model_events.rs:41-79,380` |
| UI → PTY | `PtyIntent` enum → controller → `mio_channel::Sender<Message>` | `terminal_surface.rs:18-57`, `event_loop.rs:168-179` |
| UI reads grid | direct `&GridHandler` access under the same mutex during paint | `alt_screen_element.rs:688-760` |
| surface abstraction | `trait TerminalSurface: Entity`, `trait TerminalManager { fn model() -> Arc<FairMutex<TerminalModel>> }` | `terminal_surface.rs:54`, `app/src/terminal/terminal_manager.rs:24-40` |

Platform-agnostic: parser, grid, flat storage, blocks, input encoders (parameterised by `ModeProvider`), scene/element code, wgpu renderer. Platform-specific: PTY (`unix.rs`, `windows/`), terminal server (unix), windowing/text/fonts (`warpui/src/platform/*`, `windowing/winit`), Metal renderer.

### C.2 What transfers (**Assessment**)

1. **PTY thread design**: dedicated reader thread, bounded work per lock hold, `try_lock` with fallback to blocking only under back-pressure, replies written from the same thread, child-exit funnelled into the same loop. Independent of UI technology.
2. **Payload-free, coalesced wakeups** plus a separate typed event channel. Maps directly to an FFI callback "dirty(session)" that the native side coalesces onto its display link, and a second callback for discrete events (title, bell, clipboard, exit).
3. **Narrow input vocabulary** (`PtyIntent`) and encoders that take a `ModeProvider` rather than the model: key/mouse/paste encoding stays in Rust, native UI only sends normalised events.
4. **Synchronized output handled in the parser driver** with byte and time caps, and wakeups suppressed while buffering.
5. **Scrollback as flat text + run-length attributes with stable content offsets**, making reflow an index rebuild; and absolute row coordinates that survive truncation (images, hyperlinks). The idea is reusable; the code is AGPL.
6. **Per-entry caps on untrusted input** (hyperlink URI/id/registry size, grapheme bytes, sync-output buffer, hook session-id validation).
7. **Ref tests** (recorded stream → expected grid) as the conformance harness; in our case they can live in the core crate because the core will not depend on UI.
8. **Geometry-drawn box/block/Powerline glyphs** and background run batching — renderer-side ideas that apply to a Metal/Direct2D/Win2D renderer too.
9. Conventions worth copying: tests in sibling files, clippy `-D warnings`, disallowed-types list, `report_error!`-style non-fatal invariant reporting, exhaustive matches.

### C.3 What must change (**Assessment**)

1. **No shared mutex across the FFI boundary.** Warp lets the renderer read `&GridHandler` under the lock; Swift/C# cannot hold a Rust guard safely or cheaply. Needed instead: a core-owned **snapshot/diff API** — e.g. `take_frame(session, viewport) -> Frame` returning a flat, `repr(C)`/POD buffer of rows (cell runs: text UTF-8/UTF-16 + style ids + flags), cursor, selection, and a generation number; the core copies under its own lock and releases it before returning.
2. **Add real damage tracking.** Warp has none for rendering and re-walks every visible cell at up to 60 Hz. For native UI over FFI the per-frame marshalling cost is what dominates, so per-row dirty bits (or row generation counters) kept in the core and reset by `take_frame` are required. Alacritty's `TermDamage` is the Apache-licensed reference for this.
3. **Ownership**: the core should own sessions behind opaque handles; the UI holds ids, not `Arc<Mutex<Model>>`. Warp's `TerminalManager::model()` leak of the lock to every view is the root of its documented deadlock hazard (`AGENTS.md:177-181`).
4. **Keep UI types out of the core.** `SizeInfo` with pixel units, `ColorU`, `Keystroke`, `AppContext` in `warp_terminal` would all be FFI liabilities. Core API should be cells/rows/columns + plain integers; pixel metrics belong to the native side, which reports only `(cols, rows, cell_px)` for `TIOCSWINSZ`/kitty graphics.
5. **Callbacks**: Warp's channels assume an in-process async executor on the UI thread (`spawn_stream_local`). Over FFI use a C callback (or a pollable fd / dispatch source / `DispatcherQueue` post) invoked from the PTY thread that does nothing but schedule; never call back into Swift/C# while holding the model lock.
6. **Text shaping moves to the platform.** Warp shapes inside Rust through `TextLayoutSystem`. With SwiftUI/WinUI we would shape with Core Text / DirectWrite natively, so the snapshot must carry text runs and cluster boundaries, not glyph ids. That makes grapheme-cluster-correct cells in the core mandatory (Warp's per-scalar width logic is insufficient).
7. **Model placement**: put the session model (alt screen switch, title stack, block/prompt marks) in the core crate, not the app. Warp's split forces its ref tests and both front-ends to depend on the whole app crate (`warp_tui` depends on `warp`).
8. **Shell integration**: prefer OSC 133 / OSC 7 over a private DCS JSON protocol unless blocks are a product requirement.
9. **Windows**: bundling `conpty.dll` (and OpenConsole) rather than using the in-box ConPTY is a deliberate choice in Warp; we need to decide the same question. Kitty keyboard is off on Windows there for a stated ConPTY reason (`ansi/mod.rs:1509-1536`).

### C.4 AGPL flags — places where reuse would mean copying

| Area | Warp files | Status |
| --- | --- | --- |
| Flat scrollback storage | `crates/warp_terminal/src/model/grid/flat_storage/*` | Warp-original, AGPL. Study only; design our own. |
| Block model, lifecycle, early output | `app/src/terminal/model/{blocks,block,lifecycle,early_output}.rs`, `blockgrid.rs` | AGPL. Study only. |
| Shell bootstrap scripts and hook schema | `app/assets/bundled/bootstrap/*`, `model/ansi/dcs_hooks.rs` | AGPL (scripts also embed bash-preexec under its own licence). Do not copy; the wire format is Warp-private anyway. |
| Sync-output buffering, hyperlink registry, image map, kitty graphics/keyboard code | `model/ansi/mod.rs`, `grid/hyperlink_registry.rs`, `image_map.rs`, `kitty.rs`, `escape_sequences/kitty_keyboard_protocol.rs` | AGPL. Reimplement from the public protocol specs. |
| Alacritty-derived files (event loop, unix PTY, grid storage, cell, mode, resize, ref-test harness) | see section 0 | Take from upstream `alacritty_terminal` (Apache-2.0), **not** from Warp's modified copies — Warp's changes are AGPL. |
| APC parsing | Warp `vte` fork | Fork licence not checked — **unverified**; upstream `vte` v0.13.0 has no `apc_*` callbacks (checked `alacritty/vte` `src/lib.rs` at `v0.13.0`, 2026-10-02). Implement APC handling ourselves or check newer upstream. |
| Geometry glyph tables, grid renderer | `app/src/terminal/grid_renderer.rs`, `grid_renderer/box_drawing.rs` | AGPL (in `app`). Study only. |
| Scene, atlas allocator, glyph cache, wgpu/Metal renderers, text-layout traits | `crates/warpui`, `crates/warpui_core` | **MIT** — legally reusable, though of limited use since our UI is native. |

## D. Not verified

- Kitty keyboard flags beyond 1 and 8; exact legacy mouse encodings (1000/1005/1015).
- Metal renderer internals; presenter scene lifetime (`defer_scene_build` feature exists, `crates/warpui/Cargo.toml:26`).
- Licence of the `warpdotdev/vte`, `cosmic-text`, `winit`, `font-kit`, `dwrote-rs` forks.
- `selection.rs`, `find`, `secrets` internals; remote TTY / shared-session paths.
- Runtime behaviour and performance: nothing was built or measured.
