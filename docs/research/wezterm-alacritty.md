# WezTerm and Alacritty: source-level analysis for a Rust terminal core

Date checked: 2026-10-02. All file references are `path:line` at the commits below. Anything not read in the source is marked **unverified**.

## 0. Snapshot

| Repo | Commit analysed | Commit date | Commits since 2025-10-02 | Latest release |
| --- | --- | --- | --- | --- |
| wezterm/wezterm | `cab25161054c50fd6c705db4ceefef0f1e5a9575` | 2026-09-29 | 183 | last tagged stable `20240203-110809-5046fc22` (2024-02-03); after that only the rolling `nightly` tag |
| alacritty/alacritty | `d692748d3f61253ebe9f5094320120d22f6a046f` | 2026-08-31 | 41 | `v0.17.0` (2026-04-06); before it `v0.16.1` (2025-10-20), `v0.16.0` (2025-10-18) |
| alacritty/vte | `abeae765dd546dfff60b278f0757dcc71beb8ab1` | 2026-02-28 | 1 | crate `0.15.0` (2025-02-02); GitHub "releases" page stops at `v0.13.0`, tags go to `v0.15.0` |

Sources: `git log -1` in the shallow clones; GitHub REST API `repos/<r>/releases`, `repos/<r>/tags`, `repos/<r>/commits?since=2025-10-02` (page count), `repos/<r>` (`pushed_at`, `archived=false` for all three), checked 2026-10-02.

Reading of activity:
- WezTerm: active `main` (183 commits in 12 months) but **no stable release for 2.5 years**; consumers must track git or nightly.
- Alacritty: slow, steady, roughly two releases a year; `alacritty_terminal` is published with each one.
- vte: effectively in maintenance (1 commit in 12 months), stable API.

Licenses: Alacritty `Apache-2.0` (`alacritty/alacritty_terminal/Cargo.toml:5`, `license = "Apache-2.0"`; repo ships `LICENSE-APACHE` and `LICENSE-MIT`, but the two crates declare Apache-2.0 only). vte `Apache-2.0 OR MIT` (`vte/Cargo.toml`). WezTerm `MIT` (`wezterm/LICENSE.md`, and `license = "MIT"` in each crate manifest); `wezterm-bidi` is `MIT AND Unicode-DFS-2016` (`bidi/Cargo.toml:7`).

### crates.io status (crates.io API `https://crates.io/api/v1/crates/<name>`, checked 2026-10-02)

| Crate | On crates.io | Latest | Published | License | In-repo version | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `alacritty_terminal` | yes | 0.26.0 | 2026-04-06 | Apache-2.0 | 0.26.1-dev | 44 versions since 2020-07-31; MSRV 1.85.0 |
| `vte` | yes | 0.15.0 | 2025-02-02 | Apache-2.0 OR MIT | 0.15.0 | MSRV 1.62.1; ~79M downloads |
| `vte_generate_state_changes` | yes | 0.2.0 | 2025-01-09 | Apache-2.0 OR MIT | n/a | no longer present in vte `src/` at the analysed commit |
| `alacritty_config` | yes | 0.2.3 | 2025-10-18 | MIT OR Apache-2.0 | 0.2.4-dev | not relevant for a core |
| `wezterm-term` | **no** ("crate does not exist") | n/a | n/a | MIT | 0.1.0 | git dependency only |
| `termwiz` | yes | 0.23.3 | 2025-03-20 | MIT | **0.24.0** (unpublished) | published version predates the crate split below |
| `wezterm-escape-parser` | **no** | n/a | n/a | MIT | 0.1.0 | new split crate, git only |
| `wezterm-cell` | **no** | n/a | n/a | MIT | 0.1.0 | git only |
| `wezterm-surface` | **no** | n/a | n/a | MIT | 0.1.0 | git only |
| `wezterm-char-props` | **no** | n/a | n/a | MIT | 0.1.3 | git only |
| `vtparse` | yes | 0.7.0 | 2025-04-19 | MIT | 0.7.0 | |
| `portable-pty` | yes | 0.9.0 | 2025-02-11 | MIT | 0.9.0 | previous release 0.8.1 was 2023-03-13 |
| `wezterm-bidi` | yes | 0.2.3 | 2024-01-27 | MIT AND Unicode-DFS-2016 | 0.2.3 | |
| `wezterm-input-types` | yes | 0.1.0 | 2024-01-27 | MIT | 0.1.0 | |
| `wezterm-color-types` | yes | 0.3.0 | 2024-01-27 | MIT | 0.3.0 | |
| `wezterm-dynamic` | yes | 0.2.1 | 2025-02-10 | MIT | 0.2.1 | |
| `wezterm-blob-leases` | yes | 0.1.1 | 2025-02-10 | MIT | 0.1.1 | |
| `filedescriptor` | yes | 0.8.3 | 2025-02-11 | MIT | n/a | dependency of portable-pty |
| `mux` (wezterm) | no (the crates.io `mux` is an unrelated 2016 crate) | n/a | n/a | n/a | 0.1.0, `publish = false` | |
| `wezterm-gui`, `wezterm-font` | no | n/a | n/a | n/a | `publish = false` | |

Consequence: the only way to consume WezTerm's terminal model (`wezterm-term` + `wezterm-cell` + `wezterm-surface` + `wezterm-escape-parser`) is a git dependency pinned to a commit of a large monorepo workspace (`wezterm/Cargo.toml:2-23` members; 55 `path =` workspace dependencies in the same file). Whether the published `termwiz 0.23.3` still contains its own `escape`/`cell`/`surface` modules is **unverified** (inferred from the split crates not existing on crates.io).

---

## 1. Alacritty

Workspace: `alacritty` (GUI, winit + glutin/OpenGL), `alacritty_terminal` (core), `alacritty_config`, `alacritty_config_derive` (`alacritty/Cargo.toml:1-8`). Core is ~10.9k lines of Rust in 23 files; the parser lives in the separate `vte` crate (4.1k lines, 3 files).

### 1.1 PTY I/O and threading

- **Abstraction**: `tty::EventedReadWrite` (register/reregister/deregister with a `polling::Poller`, `reader()`, `writer()`) and `tty::EventedPty: EventedReadWrite` adding `next_child_event()` (`alacritty_terminal/src/tty/mod.rs:65-96`). `tty::Options` carries shell, cwd, env, `drain_on_exit` (`tty/mod.rs:23-43`).
- **Unix**: `rustix_openpty::openpty` (`tty/unix.rs:21,196`), child set up in `pre_exec` with `setsid` and `TIOCSCTTY` (`tty/unix.rs:52,251`), signals reset to default (`tty/unix.rs:267-272`), SIGCHLD delivered through a `signal_hook` pipe that is registered in the same poller (`tty/unix.rs:282-284`), master fd set non-blocking (`tty/unix.rs:293,439`), resize via `ioctl(TIOCSWINSZ)` (`tty/unix.rs:414-417`). On macOS the shell is started through `/usr/bin/login` (`tty/unix.rs:171`).
- **Windows ConPTY**: tries to load `conpty.dll` (the one shipped with Windows Terminal) via `LoadLibraryW`/`GetProcAddress`, else falls back to the system `CreatePseudoConsole`/`ResizePseudoConsole` (`tty/windows/conpty.rs:31-88`). ConPTY pipes are blocking, so each direction gets its own thread bridged to the poller through `piper` pipes: `UnblockedReader` / `UnblockedWriter` (`tty/windows/blocking.rs:37-63,138-161`, `conpty.rs:237-238`). Child exit is watched by `ChildExitWatcher` (`tty/windows/child.rs:52-61`).
- **Threading model**: one "PTY reader" thread per terminal: `EventLoop::spawn` → `thread::spawn_named("PTY reader", …)` with a 1 MiB read buffer (`event_loop.rs:24,205-208`). The loop polls the PTY, a message channel (`Msg::Input`, resize, shutdown; `event_loop.rs:31`) and the child-exit source.
- **Ownership and locking**: the state is `Arc<FairMutex<Term<EventProxy>>>` shared by the PTY thread and the UI thread (`alacritty/src/window_context.rs:53,194`). `FairMutex` is two `parking_lot` mutexes (`data`, `next`); `lock()` takes `next` first so a waiting thread is served before the same thread can re-lock, `lease()` reserves the next turn (`alacritty_terminal/src/sync.rs:11-48`).
  - Reader: takes a lease, reads, then `try_lock_unfair()`; if the UI holds the lock it keeps reading into the buffer and only blocks once the buffer is full (`event_loop.rs:117-146`). It parses under the lock with `state.parser.advance(&mut **terminal, …)` (`event_loop.rs:154`) and gives the lock up after `MAX_LOCKED_READ = u16::MAX` bytes (`event_loop.rs:27,160`).
  - Wakeup: `Event::Wakeup` is sent to the UI unless all processed bytes are still held by a synchronized update (`event_loop.rs:166-168`).
  - Renderer: `self.terminal.lock()` (`window_context.rs:390`), copy the visible cells out, read damage, drop the lock before drawing (`alacritty/src/display/mod.rs:784-815`).
- **Core → host events**: `EventListener::send_event(Event)` with `Wakeup`, `Title`, `ClipboardStore/Load`, `ColorRequest`, `PtyWrite(String)`, `TextAreaSizeRequest`, `Bell`, `Exit`, `ChildExit`, … (`alacritty_terminal/src/event.rs:14-58,103-104`). Replies to the PTY (DA, DSR, …) go out as `Event::PtyWrite`, i.e. through the host, not written by the core itself.

Design decision: single mutex, parse on the I/O thread directly into the grid, renderer snapshot under the lock. Simple and fast; the lock is the only synchronisation point a foreign renderer has to respect.

### 1.2 Parsing (vte)

- `vte::Parser` is a table-free hand-written implementation of Paul Williams' state machine (`vte/src/lib.rs:3,28`; states `advance_csi_entry`, `advance_dcs_passthrough`, `advance_osc_string`, … at `lib.rs:190-440`). Documented deviations: UTF-8 input, OSC terminated by BEL, 7-bit codes only (`lib.rs:20-24`).
- Fast path: in ground state it uses `memchr` to find the next ESC and validates the run as UTF-8 in one go (`lib.rs:597-611`); partial UTF-8 sequences are carried across `advance` calls (`lib.rs:70-71,672-712`).
- Limits: 2 intermediates, 16 OSC params, 1024-byte OSC buffer only without `std` (heap `Vec` with `std`) (`lib.rs:46-48,63-66`). `no_std` capable (`lib.rs:30`).
- SOS/PM/APC strings are swallowed (`lib.rs:184,361,379`), so there is no APC hook for kitty graphics.
- **Parser/performer boundary, two levels**:
  1. `vte::Perform`: `print`, `execute`, `hook`/`put`/`unhook` (DCS), `osc_dispatch`, `csi_dispatch`, `esc_dispatch`, `terminated` (`lib.rs:763-825`). Raw, no semantics.
  2. `vte::ansi::Processor` + `vte::ansi::Handler` (feature `ansi`): the processor implements `Perform` and translates into ~80 semantic callbacks (`input`, `goto`, `terminal_attribute(Attr)`, `set_private_mode`, `set_hyperlink`, `push_keyboard_mode`, `clipboard_store`, …) (`vte/src/ansi.rs:495-731`, `Perform` impl from `ansi.rs:1290`: `osc_dispatch` 1329, `csi_dispatch` 1529, `esc_dispatch` 1773). `alacritty_terminal::Term` implements `Handler` (`alacritty_terminal/src/term/mod.rs:1059`).
- OSC handled: 0/2 title, 4 palette, 8 hyperlink, 10/11/12 dynamic colours, 22 mouse cursor shape, 50 cursor style, 52 clipboard, 104 reset (`ansi.rs:1350-1496`). DCS is not handled at all (`[unhandled hook]`, `ansi.rs:1311-1326`), so no sixel, no XTGETTCAP, no DECRQSS.
- **Synchronized output (2026)** is implemented inside the vte processor, opaque to the handler: on `CSI ? 2026 h` it sets a 150 ms timeout and buffers up to 2 MiB of following bytes, scanning only for BSU/ESU, then replays them on ESU/timeout/overflow (`ansi.rs:35-48,298-414,1603-1609`). The host event loop uses `sync_timeout()` as its poll timeout and calls `stop_sync` when it fires (`event_loop.rs:228-247`).

### 1.3 Terminal state

- `Term<T>` (`term/mod.rs:268-330`): `grid` and `inactive_grid` (`Grid<Cell>`), `active_charset`, `tabs: TabStops`, `mode: TermMode`, `scroll_region: Range<Line>`, `colors`, `cursor_style`, `title` + `title_stack`, `keyboard_mode_stack` + `inactive_keyboard_mode_stack`, `damage`, `config`, plus UI-ish state that lives in the core: `selection`, `vi_mode_cursor`, `is_focused`.
- Cursor: `grid::Cursor<T>` = `point`, `template: T` (the current SGR pen is literally a template cell), `charsets: Charsets([StandardCharset; 4])`, `input_needs_wrap` (pending-wrap flag) (`grid/mod.rs:33-56`). Each grid has `cursor` and `saved_cursor` (`grid/mod.rs:111-117`).
- Modes: `TermMode` bitflags, 23 bits including the five kitty-keyboard flags and UI-only `VI` (`term/mod.rs:53-86`). Private modes supported are the `NamedPrivateMode` set: 1, 3, 6, 7, 12, 25, 1000, 1002, 1003, 1004, 1005, 1006, 1007, 1042, 1049, 2004, 2026 (`term/mod.rs:1944-1992`, `vte/src/ansi.rs:938+`). No DECLRMM (left/right margins), no 1016 SGR-pixels (no match for `1016|SgrPixel` in the core or input code).
- Charsets: only `Ascii` and `SpecialCharacterAndLineDrawing` (`vte/src/ansi.rs` `StandardCharset`), G0–G3 slots.
- Tab stops: `TabStops { tabs: Vec<bool> }`, every 8 columns initially (`term/mod.rs:51,2319-2346`).
- SGR attributes: `Flags: u16` on the cell (bold, italic, dim, inverse, hidden, strikeout, five underline styles, plus layout flags `WRAPLINE`, `WIDE_CHAR`, `WIDE_CHAR_SPACER`, `LEADING_WIDE_CHAR_SPACER`) (`term/cell.rs:19-44`). No blink, no overline.

### 1.4 Screen buffer

- `Cell { c: char, fg: Color, bg: Color, flags: Flags, extra: Option<Arc<CellExtra>> }` (`term/cell.rs:141-147`); `CellExtra { zerowidth: ArrayVec<char, 9>, underline_color, hyperlink }` (`term/cell.rs:13-17,130-136`). A test pins `size_of::<Cell>() <= 24` bytes on 64-bit (`term/cell.rs:308-313`). `Color` is `Named | Spec(Rgb) | Indexed(u8)` (`vte/src/ansi.rs:1128-1132`), i.e. colours are stored unresolved.
- `Row<T> { inner: Vec<T>, occ: usize }`; `occ` is the upper bound of touched cells so resets and "is this row clear" checks are cheap (`grid/row.rs:17-25`). Rows are always full width (allocated with `columns` default cells, `grid/row.rs:35-37`).
- `Grid<T> { cursor, saved_cursor, raw: Storage<T>, columns, lines, display_offset, max_scroll_limit }` (`grid/mod.rs:110-138`).
- Alt screen: a second full `Grid` without history; `swap_alt` does `mem::swap(&mut self.grid, &mut self.inactive_grid)`, resets the alt grid on entry, swaps the keyboard mode stacks and marks full damage (`term/mod.rs:714-735`).

### 1.5 Scrollback

- `Storage<T> { inner: Vec<Row<T>>, zero, visible_lines, len }` is a ring buffer over rows: visible region and history live in the same vector, `zero` marks the bottom line (`grid/storage.rs:33-53`). Scrolling the whole screen is `rotate(-n)`, a modular add on `zero`; nothing is moved (`grid/storage.rs:15-19,180-193`).
- `Grid::scroll_up` for a region starting at line 0: bump history size, swap the fixed lines below the region, rotate, then reset the newly exposed rows in place (`grid/mod.rs:252-307`). Rows are recycled, not reallocated. Row swap is a hand-written 4-word swap (`grid/storage.rs:145-160`).
- History is allocated lazily in chunks of `MAX_CACHE_SIZE = 1000` rows (`grid/storage.rs:13,133`) and shrunk only when more than 1000 spare rows exist (`grid/storage.rs:111`).
- Limits: `Config::scrolling_history` default 10 000 (`term/mod.rs:336,359`); the GUI caps it at 100 000 (`alacritty/src/config/scrolling.rs:7,45-47`). Memory at full width: 24 B × columns × lines, uncompressed, in RAM.
- Viewport scrolling is just `display_offset` (`grid/mod.rs:129-134,163`); when not at the bottom it is advanced as new lines arrive so the view stays put (`grid/mod.rs:266-269`).

### 1.6 Unicode

- Per-cell model is **one `char` + up to 9 zero-width `char`s**. Width comes from `unicode-width` 0.2 (`alacritty_terminal/Cargo.toml` dependency list; `c.width()` at `term/mod.rs:1064`).
- Width 0 → appended to the previous cell via `push_zerowidth`, stepping back over a wide-char spacer (`term/mod.rs:1070-1084`). Width 2 → `WIDE_CHAR` cell + `WIDE_CHAR_SPACER` cell; if it does not fit at the end of the line a `LEADING_WIDE_CHAR_SPACER` is written and the glyph wraps (`term/mod.rs:1106-1130`).
- No grapheme segmentation anywhere in the repository (no match for `grapheme` in any `.rs` file). Consequences: a ZWJ emoji sequence is laid out as several independent wide cells with ZWJ attached as a zero-width char; VS16 (U+FE0F) does not widen its base; mode 2027 is not implemented. No Unicode-version negotiation.

### 1.7 Resize and reflow

- `Term::resize` (`term/mod.rs:655-705`): resize both grids, reflow only the primary (`self.grid.resize(!is_alt, …)` / `self.inactive_grid.resize(is_alt, …)`, lines 676-678), drop the selection when the column count changes, extend tab stops, reset the scroll region, resize damage.
- `Grid::resize` → `grow_lines`/`shrink_lines` then `grow_columns`/`shrink_columns` (`grid/resize.rs:14-36`). Growing lines pulls rows out of history first (keeps the cursor at the bottom) (`grid/resize.rs:40-60`).
- Reflow is driven by the `WRAPLINE` flag on the last cell of a row: `grow_columns` walks all rows bottom-up, pulling cells from the next row into a wrapped row (`grid/resize.rs:101-135`); `shrink_columns` splits rows with `Row::shrink` and carries the overflow into the following row via a `buffered` vector (`grid/resize.rs:245-275`, `grid/row.rs:53-69`). Both rebuild the entire storage (`self.raw.take_all()`), i.e. O(total cells) per column change. Cursor position is tracked through the reflow; pending-wrap is normalised by temporarily moving the cursor one past the edge (`grid/resize.rs:113-117`).

### 1.8 Input encoding

**All input encoding lives in the GUI crate, not in `alacritty_terminal`**, and is written against `winit::KeyEvent`.

- Keyboard: `alacritty/src/input/keyboard.rs`. `build_sequence(key, mods, mode)` (`keyboard.rs:295`) covers legacy xterm-style sequences and the kitty keyboard protocol with all five progressive-enhancement flags: disambiguate, event types, alternate keys, all-keys-as-escapes, associated text (`keyboard.rs:298-313,408,473`). The core only stores the flags and the push/pop stack (`term/mod.rs:320-323,1275-1324`), gated by `Config::kitty_keyboard` (`term/mod.rs:350`). The parser also understands xterm `modifyOtherKeys` (`vte/src/ansi.rs:723-728`).
- win32-input-mode (9001): not supported (no match for `9001` or an input-mode `win32` in core, input code or vte).
- Mouse: modes 1000/1002/1003, encodings X10 default, UTF-8 (1005) and SGR (1006); encoded in `alacritty/src/input/mod.rs:541-613` (limits 223 / 2015 for non-SGR at lines 577-581).
- Bracketed paste: `alacritty/src/event.rs:1369-1379`. Focus events: `alacritty/src/input/mod.rs:830-836`.

For a reuse decision this means `alacritty_terminal` gives you mode *state* but zero encoders; you would write key, mouse, paste and focus encoding yourself.

### 1.9 OSC 8 hyperlinks

`Hyperlink { inner: Arc<HyperlinkInner { id, uri }> }`; links without an explicit id get a generated `<n>_alacritty` id (`term/cell.rs:46-104`). The link is set on the cursor template (`term/mod.rs:1874-1876`) and therefore stored per cell in `CellExtra::hyperlink` (`term/cell.rs:135`). Cells of one link share one `Arc`.

### 1.10 Images

None. No sixel, iTerm2 or kitty graphics code exists (no match for `sixel|kitty graphics|iterm2|1337` in any `.rs` file; DCS and APC are dropped by the parser, see 1.2). The cell has no place to attach image data.

### 1.11 Rendering interface

- `Term::renderable_content()` → `RenderableContent { display_iter: GridIterator<Cell>, selection, cursor: RenderableCursor, display_offset, colors: &Colors, mode }` (`term/mod.rs:637-641,2393-2412`). It is a borrowed iterator over the visible cells, so it must be consumed under the lock.
- Damage: `TermDamageState { full, lines: Vec<LineDamageBounds>, last_cursor }` (`term/mod.rs:217-226`); `Term::damage()` returns `TermDamage::Full` or `Partial(iterator of {line, left, right})`, then the consumer calls `reset_damage()` (`term/mod.rs:137,178,458-491`). Per-line column bounds, viewport only; scrolling with a non-zero display offset degrades to full damage (comment at `term/mod.rs:482-483`).
- GUI side: `Display::draw` copies cells into `Vec<RenderableCell>` (resolved RGB, flags, `zerowidth`, hyperlink) while holding the lock, pulls damage, drops the lock (`alacritty/src/display/mod.rs:775-815`, `display/content.rs:189-206`). Colour resolution (named/indexed → RGB, dim, inverse) happens in the GUI crate (`display/content.rs:310-388`). A second-level `DamageTracker` keeps two frames of damage for `swap_buffers_with_damage` (`display/damage.rs:16-58`, `display/mod.rs:607-618`).
- Glyphs: `GlyphCache { cache: HashMap<GlyphKey, Glyph>, rasterizer: crossfont::Rasterizer, … }` (`renderer/text/glyph_cache.rs:46-51`), 1024×1024 row-packed atlases, a new atlas is appended when one fills (`renderer/text/atlas.rs:12,33,225-268`). One glyph per `char`; **no shaping, no ligatures** (no match for `ligature|harfbuzz|shaping` in the repo). Box-drawing glyphs are built in (`renderer/text/builtin_font.rs`).
- Synchronized output: see 1.2; the renderer simply is not woken while bytes are held (`event_loop.rs:166-168`).

---

## 2. WezTerm

Relevant crates (all `MIT`): `vtparse` (state machine) → `wezterm-escape-parser` (semantic `Action`s) → `wezterm-cell` (Cell/attributes/images/width) → `wezterm-surface` (Line, clusters, hyperlink rules) → `wezterm-term` (`term/`, the emulator) → `mux` (panes, PTY threads) → `wezterm-gui`. `termwiz` 0.24 is now mostly a facade re-exporting the split crates (`pub use wezterm_escape_parser as escape`, `pub use wezterm_surface as surface`, `pub use wezterm_cell as cell`; `termwiz/src/lib.rs:44-68`) plus its own TUI-side code (`input`, `terminal`, `lineedit`, `widgets`, `caps`).

### 2.1 PTY I/O and threading

- `portable-pty` (`pty/`): traits `PtySystem::openpty(PtySize) -> PtyPair`, `MasterPty` (`resize`, `get_size`, `try_clone_reader`, `take_writer`, `process_group_leader`, `tty_name`), `SlavePty::spawn_command(CommandBuilder)`, `Child`, `ChildKiller`; entry point `native_pty_system()` (`pty/src/lib.rs:90-167,255-271,400`). Blocking `Read`/`Write` objects, no poller integration.
  - Unix: `libc::openpty`, `pre_exec` with `setsid`, `TIOCSCTTY`, `close_random_fds` (`pty/src/unix.rs:22-46,238-276`), resize via `TIOCSWINSZ` (`unix.rs:180-197`).
  - Windows: `CreatePseudoConsole` loaded through `shared_library`, preferring a sideloaded `conpty.dll`/`OpenConsole.exe` (`pty/src/win/pseudocon.rs:31-61`), created with `PSEUDOCONSOLE_INHERIT_CURSOR | PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE` (`pseudocon.rs:25-29,81-87`). Also a serial-port backend (`pty/src/serial.rs`).
- **Threads per pane** (`mux/src/lib.rs`): `Mux::add_pane` spawns `read_from_pane_pty` (`lib.rs:782-800`), which does blocking 1 MiB reads from the PTY and forwards bytes over a socketpair (`lib.rs:118,270-273,283-330`) to a second thread running `parse_buffered_data` (`lib.rs:142-247,315-318`). The parser thread owns its own `Parser`, produces `Vec<Action>`, coalesces for `mux_output_parser_coalesce_delay_ms` to catch a full TUI frame (`lib.rs:148,198-231`), then calls `pane.perform_actions(actions)` and emits `MuxNotification::PaneOutput` (`lib.rs:122-128`).
- **Ownership and locking**: `LocalPane { terminal: Mutex<Terminal>, pty: Mutex<Box<dyn MasterPty>>, writer: Mutex<…>, … }` (`mux/src/localpane.rs:126-129`, `parking_lot`). Every accessor locks per call: `perform_actions` (`localpane.rs:390-391`), `get_changed_since` (186-191), `with_lines_mut` (206-207), `key_down` (408), `resize` (417-424). Parsing happens **outside** the terminal lock; only applying actions holds it.
- Writes to the PTY go through a third thread: `TerminalState.writer` is a `BufWriter<ThreadedWriter>` that sends to a channel drained by a spawned thread, so key input and answerbacks never block the model (`term/src/terminalstate/mod.rs:370,455-508`). Unlike Alacritty, the core writes replies itself via the `Write` handed to `Terminal::new` (`term/src/terminal.rs:145-152`).

Design decision: parse and apply are decoupled by an owned `Action` stream. That costs allocation per action but gives a serialisable boundary (the same actions travel over the mux protocol) and keeps lock hold times short.

### 2.2 Parsing

- `vtparse::VTParser`: table-driven Williams state machine (`vtparse/src/lib.rs:2,70-75`, tables in `vtparse/src/transitions.rs`), UTF-8 via `utf8parse` (`lib.rs:17,379`), adds an **APC** state (`vtparse/src/enums.rs:22-24,51`). Actor trait `VTActor`: `print`, `execute_c0_or_c1`, `dcs_hook/put/unhook`, `esc_dispatch`, `csi_dispatch(&[CsiParam], truncated, byte)`, `osc_dispatch`, `apc_dispatch` (`lib.rs:92-185`). Limits: 2 intermediates, 64 OSC params, 256 CSI params (`lib.rs:313-315`). `no_std` (`lib.rs:15`).
- `wezterm_escape_parser::parser::Parser::parse(bytes, FnMut(Action))` (`wezterm-escape-parser/src/parser/mod.rs:67,95`) lifts raw events into a fully typed enum:
  `Action::{Print(char), PrintString(String), Control(ControlCode), DeviceControl(DeviceControlMode), OperatingSystemCommand(Box<…>), CSI(CSI), Esc(Esc), Sixel(Box<Sixel>), XtGetTcap(Vec<String>), KittyImage(Box<KittyImage>)}` (`wezterm-escape-parser/src/lib.rs:47-66`). CSI alone is 3.4k lines of typed variants (`csi.rs`), OSC 2k (`osc.rs`), kitty graphics 1.3k (`apc.rs`), sixel has its own sub-parser (`parser/sixel.rs`). Actions can be re-encoded to bytes (`Display` impls, `lib.rs:117`; encode tests at `parser/mod.rs:364,558-705`).
- **Parser/performer boundary**: a data type, not a trait. `Terminal::advance_bytes` = `parser.parse(bytes, |action| performer.perform(action))` (`term/src/terminal.rs:164-174`); `Terminal::perform_actions(Vec<Action>)` accepts pre-parsed actions (`terminal.rs:176-185`). `Performer::perform` matches on the enum (`term/src/terminalstate/performer.rs:252-288`). Printable chars are buffered and flushed as a string before any non-print action so graphemes can be segmented (`performer.rs:282,378,493,587,743`).
- The crate is `no_std`-capable with `alloc` (`wezterm-escape-parser/src/lib.rs:10`), optional features `image`, `kitty-shm`, `tmux_cc`, `use_serde` (`wezterm-escape-parser/Cargo.toml` `[features]`).

### 2.3 Terminal state

- `Terminal { state: TerminalState, parser: Parser }` derefs to `TerminalState` (`term/src/terminal.rs:85-104`).
- `TerminalState` (`term/src/terminalstate/mod.rs:255-409`): `screen: ScreenOrAlt`, `pen: CellAttributes`, `cursor: CursorPosition {x, y, shape, visibility, seqno}` (`term/src/lib.rs:107-113`), `wrap_next`, `insert`, `dec_auto_wrap`, `reverse_wraparound_mode`, `reverse_video_mode`, `dec_origin_mode`, `top_and_bottom_margins`, **`left_and_right_margins` + `left_and_right_margin_mode` (DECLRMM)**, `application_cursor_keys`, `modify_other_keys`, `application_keypad`, `bracketed_paste`, mouse flags + `mouse_encoding`, `focus_tracking`, `keyboard_encoding`, `g0_charset`/`g1_charset`/`shift_out`, `newline_mode`, `tabs: TabStop`, titles, `progress`, `palette`, `current_dir` (OSC 7), `user_vars`, `image_cache`, `kitty_img`, `seqno`, `unicode_version` + stack, `enable_conpty_quirks`, bidi flags.
- Modes as individual bools rather than a bitset. `DecPrivateModeCode` covers 1, 2, 3, 4, 5, 6, 7, 8, 12, 25, 45, 47, 69, 80, 1000–1006, 1016, 1036, 1039, 1047, 1048, 1049, 1070, 2004, 2026, 2027, 7727, 8452, 9001 (`wezterm-escape-parser/src/csi.rs`, enum ending at `SynchronizedOutput = 2026` line 957).
- Charsets: `CharSet::{Ascii, Uk, DecLineDrawing}`, G0/G1 with shift in/out (`terminalstate/mod.rs:53-62,340-342`).
- Tab stops: `TabStop { tabs: Vec<bool>, tab_width }` (`terminalstate/mod.rs:48-139`).
- Saved cursor per screen (`term/src/screen.rs:47`); kitty keyboard stack per screen (`screen.rs:39`).
- Host callbacks are trait objects set on the state: `Clipboard`, `DeviceControlHandler`, `AlertHandler` (bell, title, palette, progress, user var, toast, …), `DownloadHandler` (`term/src/terminal.rs:13-82`, setters at `terminalstate/mod.rs:622-634`). Configuration is a trait object `TerminalConfiguration` where only `color_palette()` is required, everything else has defaults (`term/src/config.rs:135-227`).
- Shell integration is modelled: `SemanticType` per cell (prompt/input/output, OSC 133) (`wezterm-cell/src/lib.rs:184,214`) and `get_semantic_zones` (`terminalstate/mod.rs:2831`).

### 2.4 Screen buffer

- `Cell { text: TeenyString, attrs: CellAttributes }`, 24 bytes (`wezterm-cell/src/lib.rs:719-729`; sizes asserted at `lib.rs:1030-1036`: `CellAttributes` 16, `TeenyString` 8, `Cell` 24).
  - `TeenyString(u64)`: grapheme bytes inline when shorter than 8 bytes and width < 3, with marker bits for "inline" and "double width"; otherwise a heap pointer to `{bytes, width}` (`lib.rs:548-565,623`).
  - `CellAttributes { attributes: u32 bitfield, foreground: SmallColor, background: SmallColor, fat: Option<Box<FatAttributes>> }` (`lib.rs:58-68`). Bitfield: intensity, underline (3 bits), blink (2 bits), italic, reverse, strikethrough, invisible, wrapped, overline, semantic_type, vertical_align (`lib.rs:205-215`). `FatAttributes { hyperlink: Option<Arc<Hyperlink>>, image: Vec<Box<ImageCell>>, underline_color, foreground, background }` holds the rare stuff, including true-colour values (`lib.rs:93-104`).
- `Line { cells: CellStorage, zones: Vec<ZoneRange>, seqno: SequenceNo, bits: LineBits, appdata: Mutex<Option<Weak<dyn Any>>> }` (`wezterm-surface/src/line/line.rs:46-54`). `LineBits`: hyperlink flags, double-width / double-height (DECDWL/DECDHL), bidi enabled, RTL, auto-detect direction (`line/linebits.rs:8-52`). `appdata` is a slot where the renderer caches per-line data (`line.rs:256-273`).
- **Two storage forms** (`line/storage.rs:9-12`): `CellStorage::V(VecStorage)` (a `Vec<Cell>`, mutable form) and `CellStorage::C(ClusteredLine)` (compact form): `ClusteredLine { text: String, is_double_wide: Option<Box<FixedBitSet>>, clusters: Vec<Cluster { cell_width: u16, attrs }>, len: u32, last_cell_width }` (`line/clusterline.rs:18-42`): one contiguous string plus runs of identical attributes. Lines are variable length; trailing blanks are not stored.
- `Screen { lines: VecDeque<Line>, stable_row_index_offset, config, allow_scrollback, keyboard_stack, physical_rows, physical_cols, dpi, saved_cursor }` (`term/src/screen.rs:16-48`). The visible screen is the last `physical_rows` entries of `lines`.
- Alt screen: `ScreenOrAlt` holds two `Screen`s and an `alt_screen_is_active` flag, derefs to the active one (`terminalstate/mod.rs:152-253`); the alt `Screen` is built with `allow_scrollback = false` (`screen.rs:36-37,50-56`).
- Row addressing: `PhysRowIndex` (index into `lines`), `VisibleRowIndex` (i64, viewport), `StableRowIndex` (isize, survives scrollback eviction via `stable_row_index_offset`) (`term/src/lib.rs:47-80`, `screen.rs:523-529`). Stable indices are what the renderer, selection and semantic zones use.

### 2.5 Scrollback

- Same `VecDeque<Line>` as the screen, capacity `physical_rows + scrollback_size` (`screen.rs:50-56,73`). Default `scrollback_lines = 3500` (`config/src/config.rs:210,1705-1707`).
- `Screen::scroll_up` (`screen.rs:646-761`): when the region starts at row 0 and scrollback is allowed, the lines leaving the viewport are **compressed in place** with `Line::compress_for_scrollback()` (`screen.rs:690-694`, `line.rs:1069-1075`) and stay in the deque; when over capacity, lines are removed from the front and `stable_row_index_offset` advances (`screen.rs:682-735`). Lines removed from the top are reused at the bottom "to avoid thrashing the heap" (`screen.rs:703-722`).
- Scroll cost: for the normal full-screen case it is `push_back` + occasional `pop_front`; because `StableRowIndex` of surviving rows does not change, they are **not** marked dirty (`screen.rs:663-673`). Scrolling inside a region uses `VecDeque::remove`/`insert` at an index (O(n) in deque length) and dirties the affected rows (`screen.rs:708-720,755-760`).
- Trade-off against Alacritty's ring: more per-line work at scroll time (cluster compression) in exchange for much smaller history lines.

### 2.6 Unicode

- **Grapheme per cell.** `Performer::flush_print` optionally NFC-normalises the buffered text (`performer.rs:124-132`), then iterates `finl_unicode::grapheme_clusters::Graphemes` (`performer.rs:6,134`), computes `grapheme_column_width(g, Some(&self.unicode_version))` (`performer.rs:137`), and stores the whole grapheme in one cell with `set_cell_grapheme` (`performer.rs:223-224`). Zero-width graphemes are turned into a space if they are White_Space, otherwise dropped (`performer.rs:138-162`).
- Width (`wezterm-cell/src/lib.rs:945-984`): single-byte fast path; for `version >= 14`, emoji presentation including variation selectors decides the width (`Presentation::for_grapheme`: VS16 → 2, VS15 → 1); otherwise sum of per-char widths clamped to 2. Tables come from `wezterm-char-props` (generated `widechar_width.rs` "for Unicode 16.0.0", `wezterm-char-props/src/widechar_width.rs:2`; emoji presentation/variation tables in `emoji_presentation.rs`, `emoji_variation.rs`).
- **Unicode version handling**: `UnicodeVersion { version: u8, ambiguous_are_wide: bool, cell_widths: Option<Arc<HashMap<u32, u8>>> }` (`wezterm-cell/src/lib.rs:832-837`): pre/post Unicode 9 width tables, CJK-ambiguous switch, user override map (`lib.rs:850-874`). The terminal keeps a current version and a push/pop stack driven by an OSC (`terminalstate/mod.rs:381-382,412`, `performer.rs:839-852`), with the config default from `TerminalConfiguration::unicode_version()` (`term/src/config.rs:190`). `LATEST_UNICODE_VERSION.version = 14` (`wezterm-cell/src/lib.rs:882-887`).
- Mode 2027 (grapheme clustering) is reported as permanently set (`terminalstate/mod.rs:1624-1637`).
- Bidi: `wezterm-bidi` (UBA implementation, `no_std`, `bidi/src/lib.rs:1`) is applied at clustering time: `Line::cluster(bidi_hint)` → `CellCluster { attrs, text, width, presentation, direction, first_cell_idx, … }` (`wezterm-surface/src/line/line.rs:1043`, `cellcluster.rs:18-27,158`). Per-line bidi flags are set from the escape-sequence-controlled mode (`terminalstate/mod.rs:404-408`).

### 2.7 Resize and reflow

- `TerminalState::resize(TerminalSize { rows, cols, pixel_width, pixel_height, dpi })` bumps the seqno and resizes both screens, tracking both cursors (`terminalstate/mod.rs:972-1000`, `term/src/terminal.rs:108-114`).
- `Screen::resize` (`screen.rs:194-300`): prune blank lines below the cursor; if the width changed and this is the primary screen, `rewrap_lines`; the alt screen is only truncated or invalidated (`screen.rs:226-247`); then pad to the viewport height.
- `rewrap_lines` (`screen.rs:101-191`): drain all lines, join physical lines whose last cell has the `wrapped` attribute into one logical line (`Line::append_line`), re-split with `Line::wrap(width, seqno)` (`line.rs:214`), track the cursor's logical x through join/split (`screen.rs:122-166`), drop trailing blank lines if over capacity. O(total cells), every line gets a new seqno.
- ConPTY quirk: with ConPTY, resize treats scrollback as immutable (`resize_preserves_scrollback = is_conpty`, `screen.rs:268-288`) and the wrapped flag is set only when the last cell looks like text (`performer.rs:175-192`), because ConPTY repaints on resize.

### 2.8 Input encoding

- Entry points live **in the core**: `TerminalState::key_down/key_up(KeyCode, KeyModifiers)` (`term/src/terminalstate/keyboard.rs:60-66`), `mouse_event(MouseEvent)` (`terminalstate/mouse.rs:322`), `send_paste(&str)` (`terminalstate/mod.rs:943-966`), `focus_changed(bool)` (`terminalstate/mod.rs:887-915`). Types come from `wezterm-input-types`, independent of any windowing toolkit.
- Legacy / CSI-u / modifyOtherKeys: `KeyCode::encode(mods, KeyCodeEncodeModes { encoding, newline_mode, application_cursor_keys, modify_other_keys }, is_down)` in `termwiz/src/input.rs:241` (`KeyboardEncoding::{Xterm, CsiU, Win32, Kitty(flags)}` at `input.rs:21-28`).
- **Kitty keyboard**: all five flags defined (`wezterm-input-types/src/lib.rs:2035-2041`), encoder `KeyEvent::encode_kitty(flags)` (`lib.rs:1711`), push/pop stack per screen (`screen.rs:39`, `performer.rs:523-563`). Off by default: `enable_kitty_keyboard` is `#[dynamic(default)]` = false (`config/src/config.rs:258-259`). Note the kitty and win32 encoders are invoked from the GUI (`wezterm-gui/src/termwindow/keyevent.rs:192-205`), which writes the result to the pane; `key_down` in the core handles the Xterm/CsiU path. The encoders themselves are in the reusable `wezterm-input-types` crate.
- **win32-input-mode** (9001): supported: mode switch at `terminalstate/mod.rs:1571-1583`, encoder `KeyEvent::encode_win32_input_mode()` (`wezterm-input-types/src/lib.rs:1642-1648`), ConPTY created with `PSEUDOCONSOLE_WIN32_INPUT_MODE` (`pty/src/win/pseudocon.rs:87`); `allow_win32_input_mode` defaults true (`config/src/config.rs:873-874`).
- Mouse: tracking 1000/1002/1003, encodings `MouseEncoding::{X10, Utf8, SGR, SgrPixels}` (1005/1006/1016) (`terminalstate/mod.rs:66-71`, `terminalstate/mouse.rs:8-322`).
- Bracketed paste strips embedded `ESC[200~`/`ESC[201~` from the pasted text (`terminalstate/mod.rs:956`). Focus loss also synthesises button releases (`terminalstate/mod.rs:891-906`).

### 2.9 OSC 8 hyperlinks

`Hyperlink { params: HashMap<String,String>, uri, implicit: bool }` (`wezterm-escape-parser/src/hyperlink.rs:11-17`), stored as `Option<Arc<Hyperlink>>` in the cell's `FatAttributes` (`wezterm-cell/src/lib.rs:95,356,468`), set via the pen (`terminalstate/mod.rs:1300-1301`, `performer.rs:773-774`). Lines carry `HAS_HYPERLINK` bits for fast checks (`line/linebits.rs:11`). **Implicit hyperlinks**: regex rules applied per logical line produce `implicit` links (`line.rs:488-571`, `hyperlink.rs:50-54`); this is why `wezterm-surface` depends on `fancy-regex`.

### 2.10 Images

- Sixel: parsed to `Action::Sixel(Box<Sixel>)` (`wezterm-escape-parser/src/lib.rs:62,287,408`, `parser/sixel.rs`), rendered in `terminalstate/sixel.rs:10`; DECSDM and private colour registers are tracked (`terminalstate/mod.rs:311,373`).
- iTerm2 `OSC 1337 File=`: `terminalstate/iterm.rs:10` (`ITermFileData`); non-inline files go to the `DownloadHandler`.
- Kitty graphics: `Action::KittyImage` from APC; transmit / transmit+display / display / query / delete / animation frame transmit and compose (`terminalstate/kitty.rs:153-284,390-543`), direct, file and shared-memory transfer (feature `kitty-shm`). Gaps visible in the source: some delete targets only log `unhandled KittyImage::Delete` (`kitty.rs:270-271`); a 320 MiB data budget is hard-coded (`kitty.rs:47`); Unicode placeholders (U+10EEEE) do not appear anywhere in `term/src` or `apc.rs` (no match). Enabled by default (`config/src/config.rs:256-257`).
- **Attachment model**: every image is sliced into cells. `assign_image_to_cells` computes, for each covered cell, an `ImageCell { top_left, bottom_right: TextureCoordinate, data: Arc<ImageData>, z_index, padding_{left,top,right,bottom}, image_id, placement_id }` and stores it in that cell's `FatAttributes::image` (`terminalstate/image.rs:65-283`, `wezterm-cell/src/image.rs:92-114`). Kitty placements are appended (`attach_image`, several per cell with z-index), sixel/iTerm replace (`set_image`) (`image.rs:237-241`). Images therefore scroll, reflow and get erased with the text for free. `ImageData { data: Mutex<ImageDataType>, hash: [u8;32] }` is deduplicated through an LRU keyed by SHA-256 (`terminalstate/mod.rs:372,583` (16 entries), `terminalstate/image.rs:286-296`), and can be spooled to disk through `wezterm-blob-leases`.

### 2.11 Rendering interface

- **Change tracking by sequence number.** `SequenceNo = usize` (`wezterm-surface/src/lib.rs:90`). `advance_bytes`/`perform_actions`/`resize` bump `TerminalState.seqno` (`terminal.rs:165,177`, `terminalstate/mod.rs:973`); every mutating `Line` method takes the seqno and stamps the line (`line.rs:295`); `Line::changed_since(seqno)` (`line.rs:283`). `Screen::get_changed_stable_rows(range, seqno)` (`screen.rs:910-929`). Granularity is a whole line, no column bounds; any consumer can hold its own "last seen" seqno, so multiple independent viewers work.
- **`mux::Pane` trait** (`mux/src/pane.rs:172-349`) is the renderer-facing abstraction: `get_cursor_position() -> StableCursorPosition`, `get_current_seqno()`, `get_changed_since(range, seqno) -> RangeSet<StableRowIndex>`, `get_lines(range) -> (StableRowIndex, Vec<Line>)`, `with_lines_mut(range, &mut dyn WithPaneLines)`, `get_logical_lines`, `get_dimensions() -> RenderableDimensions { cols, viewport_rows, scrollback_rows, physical_top, scrollback_top, dpi, pixel_*, reverse_video }`, plus input (`key_down`, `mouse_event`, `send_paste`, `resize`, `writer`). Helpers that implement these for a `Terminal` are in `mux/src/renderable.rs:51-141`. The viewport scroll position is owned by the GUI, not the core; the GUI asks for any stable row range.
- GUI: `paint_pane` calls `pane.with_lines_mut(stable_range, &mut render)` (`wezterm-gui/src/termwindow/render/pane.rs:558-568`), i.e. renders while the terminal mutex is held. Per line it computes a `shape_hash` and looks up a `LineQuadCacheKey` (pane, selection, cursor, shape hash, pixel position, generations) in `line_quad_cache` (`pane.rs:424-446`); on miss it clusters and shapes: `line.cluster(bidi_hint)` → `cached_cluster_shape` (`render/screen_line.rs:740-929`), with a second cache `line_to_ele_shape_cache` (`screen_line.rs:119-134,913-918`).
- Fonts: `wezterm-font` with `FontShaper` (HarfBuzz, configurable `harfbuzz_features`, `wezterm-font/src/shaper/mod.rs:116-118`, `shaper/harfbuzz.rs:127-140`) and `FontRasterizer` (FreeType / HarfBuzz COLR, `rasterizer/mod.rs:31-32`). Ligatures and complex scripts are supported through shaping.
- Glyph cache: `GlyphCache { glyph_cache: HashMap<GlyphKey, Rc<CachedGlyph>>, atlas: Atlas, image_cache: LfuCache<[u8;32], DecodedImage>, frame_cache, line_glyphs, block_glyphs, cursor_glyphs, color }`; eviction is "recreate everything when the atlas fills" (`wezterm-gui/src/glyphcache.rs:558-569`). Images are uploaded into the same atlas (`glyphcache.rs:1084-1105`). Backends: OpenGL, WebGPU, Software (`config/src/frontend.rs:7-9`).
- **Synchronized output (2026)** is implemented in the mux parser thread, not in the core: on `SetDecPrivateMode(SynchronizedOutput)` it flushes and sets `hold`, accumulating actions until the reset (or a soft reset) (`mux/src/lib.rs:162-196`). `wezterm-term` itself ignores the mode and reports it as not set (`terminalstate/mod.rs:1693-1709`). **There is no timeout** in this code path: while `hold` is true nothing is applied until the reset arrives (`lib.rs:198`). Anyone embedding `wezterm-term` without `mux` has to re-implement this.

---

## 3. Core/GUI separation and fit for a foreign (non-Rust) renderer

### 3.1 Alacritty: `alacritty_terminal` vs `alacritty`

What the core gives you: PTY (unix + ConPTY), a poll-based I/O thread, parser, `Term` state, grid + scrollback + reflow, selection, regex search (`term/search.rs`), vi-mode cursor, line damage. Dependencies are small and all pure Rust: `vte`, `parking_lot`, `polling`, `regex-automata`, `unicode-width`, `bitflags`, `arrayvec`, `base64`, `home`, `log`, `rustix-openpty`, `signal-hook` / `windows-sys`, `miow`, `piper` (`alacritty_terminal/Cargo.toml`). No GUI, font or config-file types leak in; the host supplies only an `EventListener` and a `Dimensions`.

What stays in the GUI crate and must be rebuilt: all input encoding (keyboard incl. kitty protocol, mouse, paste, focus; section 1.8), colour resolution to RGB (`display/content.rs:310-388`), URL/hint detection (`display/hint.rs`), glyph rasterisation.

FFI fit:
- Good: the read side is tiny and flat. Under one lock: iterate `display_iter`, each cell is `(char, fg, bg, flags, optional extra)`; cursor point + shape; `damage()` gives `{line, left, right}` ranges. A C ABI "copy damaged rows into a caller buffer of 16–24-byte POD cells" is straightforward, and the fixed-width rows map directly to a buffer.
- Bad: `RenderableContent` borrows the `Term`, so the FFI must copy under the lock (the same thing Alacritty's own GUI does). Zero-width chars and hyperlinks sit behind `Option<Arc<CellExtra>>` and need a side channel. Text arrives as isolated `char`s: a CoreText/DirectWrite renderer has to re-assemble runs itself and cannot get correct emoji-ZWJ or VS16 widths, because the grid already committed to wrong cell widths.
- API stability: `0.x` with a breaking minor per Alacritty release (0.25 → 0.26 within six months). The crate is published "as is" for Alacritty's needs.

### 3.2 WezTerm: `wezterm-term` / `termwiz` / `mux` vs `wezterm-gui`

`wezterm-term` is explicitly designed as an embeddable model: "does not provide any kind of gui, nor does it directly manage a PTY; you provide a `std::io::Write` … and supply bytes via `advance_bytes`" (`term/src/lib.rs:11-14`). It needs four things from the host: a `TerminalSize`, an `Arc<dyn TerminalConfiguration>` (one required method), program name/version, and a writer (`term/src/terminal.rs:145-152`). Input encoding, images, hyperlinks, semantic zones and bidi are inside.

What is *not* in `wezterm-term` and lives in `mux`: PTY threads, parse coalescing, synchronized output, the `Pane` trait. `mux` is `publish = false` and is wired to WezTerm's config, Lua helper, domain/tab/window model, SSH and tmux integration (`mux/src/renderable.rs:2` imports `luahelper`; `mux/src/lib.rs:1-9` imports `config`, `domain`, `ssh_agent`, `tab`, `window`). It is not reusable as a library; its ~150 lines of reader/parser threads are the part worth copying.

Dependency weight of `wezterm-term`: `termwiz` (with `use_image`), `image`, `terminfo`, `url`, `lru`, `csscolorparser`, `unicode-normalization`, `finl_unicode`, `miniz_oxide`, `humansize`, `anyhow`, `wezterm-bidi`, `wezterm-dynamic`, `wezterm-cell`, `wezterm-escape-parser`, `wezterm-surface` (`term/Cargo.toml` `[dependencies]`); `wezterm-surface` adds `fancy-regex`, `unicode-segmentation`, `siphasher` and `wezterm-input-types`; `wezterm-cell` itself depends on `wezterm-escape-parser` (crate manifests). `termwiz` defaults pull in `pest` (tmux control mode; `termwiz/Cargo.toml:51,56`) and, on Unix, `termios`/`signal-hook`/`nix` (`termwiz/Cargo.toml:64-67`). Usable, but a much larger tree than Alacritty's, and none of the central crates is versioned on crates.io.

FFI fit:
- Good: `Line::cluster()` yields `CellCluster { text: String, attrs, width, direction, first_cell_idx }`, which is exactly the shape a CoreText / DirectWrite text-run API wants: runs of same-attribute text with a byte→cell map. Seqno-based change tracking is pull-style and stateless on the core side: the foreign renderer stores one integer and asks "which stable rows changed since N". `StableRowIndex` gives the UI a robust scroll model.
- Bad: no flat cell buffer. `Cell` holds a tagged pointer (`TeenyString`) and `Option<Box<FatAttributes>>`; lines are variable-length and may be in clustered form. An FFI layer has to serialise per line (text + attribute runs), which is more code than Alacritty's copy-out but is also the more natural payload for a native text stack. Image cells carry `Arc<ImageData>` behind a mutex; the FFI needs an image-handle table.
- Synchronized output and PTY threading must be re-implemented by the embedder (section 2.11).

### 3.3 Verdict on the boundary

| | Alacritty | WezTerm |
| --- | --- | --- |
| Core crate published and versioned | yes (`alacritty_terminal` 0.26.0) | no (git only) |
| Core includes PTY + I/O thread | yes | no (separate `portable-pty`; threads in unreusable `mux`) |
| Core includes input encoding | no | yes |
| Renderer contract | borrowed cell iterator + line/column damage | lines by stable index + per-line seqno; `Line::cluster()` text runs |
| Data shape for FFI | fixed-size cells, easy to memcpy | per-line text + attribute runs, needs serialisation |
| Text model | `char` + zero-width extras | grapheme cluster per cell |
| Features a modern terminal needs that are missing | graphemes, images, DECLRMM, win32-input, shaping-friendly runs, OSC 133 | sync-update timeout in core, published crates, stable API |

---

## 4. Reuse recommendation

Requirements assumed from the brief: Rust core computes, SwiftUI/WinUI render through FFI; images, kitty keyboard and correct Unicode are expected features.

| Crate | Verdict | Reasons |
| --- | --- | --- |
| `vte` 0.15 (`Parser` + `Perform`) | **Use as-is** (if we write our own state) | Published, dual MIT/Apache, tiny, `no_std`, fastest path for plain text (memchr + bulk UTF-8). Low activity is acceptable for a finished state machine. Limitation: APC is dropped, so kitty graphics needs either a patch or a pre-filter; DCS is exposed raw via `hook/put/unhook`, so sixel is possible on top. |
| `vte::ansi` (`Processor` + `Handler`) | **Learn from; optionally use** | Convenient typed callbacks and built-in 2026 handling with timeout, but the `Handler` vocabulary is exactly Alacritty's feature set (no DCS, no APC, no DECLRMM, no OSC 133/1337/7). Extending it means forking vte. |
| `alacritty_terminal` 0.26 | **Learn from; fork only if images and graphemes are dropped from scope** | Best-engineered small core, published, clean host boundary, flat cells ideal for FFI. But the data model is the blocker: `char`-per-cell with no grapheme clustering, no image attachment point, no input encoders, no left/right margins, no win32-input-mode. Adding graphemes or images changes `Cell`, `Handler::input`, reflow and selection, which is a deep fork that will not merge upstream. Apache-2.0 only (fine, but note it is not dual-licensed like vte). Take from it: `Storage` ring buffer, `FairMutex` + lease read loop, line damage bounds, ConPTY reader/writer threads. |
| `vtparse` 0.7 | **Use as-is** (alternative to `vte`) | Published, MIT, `no_std`, has APC and 256 CSI params with colon sub-parameter support (`CsiParam::ColonList`, `vtparse/src/lib.rs:389-405`). Per-byte table dispatch, no bulk fast path (slower on plain text; **unverified** by measurement). Pick this if we adopt `wezterm-escape-parser`. |
| `wezterm-escape-parser` 0.1.0 | **Fork/vendor (git pin), or learn from** | The most complete typed escape model available in Rust (CSI/OSC/DCS/APC, sixel, kitty graphics, round-trip encoding), MIT, `no_std + alloc`. Not on crates.io; depends on `wezterm-dynamic`, `wezterm-input-types`, `wezterm-color-types`, `wezterm-blob-leases`. Allocates per `Action`. Vendoring a pinned copy is realistic because the crate is self-contained and data-only. |
| `wezterm-term` 0.1.0 | **Fork (git pin) if we want a feature-complete core quickly; otherwise learn from** | Only Rust core that already has graphemes + Unicode version stack, sixel/iTerm2/kitty images, kitty keyboard + win32-input, SGR-pixel mouse, DECLRMM, OSC 8 + implicit links, OSC 133 zones, bidi flags. MIT. Costs: not published (pin a commit of a monorepo with no stable release since 2024-02), heavy dependency tree, API shaped around WezTerm's mux, `VecDeque::remove/insert` region scrolling, no sync-output timeout, config via trait object with WezTerm semantics. A fork means owning ~30k lines across `term`, `wezterm-cell`, `wezterm-surface`, `wezterm-escape-parser`. |
| `wezterm-cell` / `wezterm-surface` | **Learn from** (come along if `wezterm-term` is forked) | Ideas worth copying into our own grid: 24-byte cell with inline grapheme (`TeenyString`) and boxed "fat" attributes; two-form line storage (mutable vec vs clustered text + attribute runs) for cheap scrollback; per-line seqno; `StableRowIndex`. Not usable standalone from crates.io. |
| `termwiz` 0.23.3 (crates.io) / 0.24 (git) | **Do not use in the core** | It is a TUI-application toolkit (terminal capabilities, line editor, widgets) that happens to re-export the model crates. The crates.io version lags the repo and the split. Take `KeyCode::encode` logic as reference for legacy key encoding. |
| `wezterm-input-types` 0.1.0 | **Learn from / vendor the encoders** | Toolkit-independent `KeyEvent` with `encode_kitty` and `encode_win32_input_mode`, MIT, published, but the crates.io release is from 2024-01-27 while the repo copy has moved on. Good reference for our own FFI key event type. |
| `portable-pty` 0.9.0 | **Use as-is** | Published (2025-02-11), MIT, the de-facto cross-platform PTY crate (17.98M downloads per crates.io API), handles ConPTY incl. sideloaded `conpty.dll` and the win32-input flag. Caveats: blocking reader/writer (we need our own reader thread, same as WezTerm), uses `anyhow` in its API, 2-year gap between 0.8.1 and 0.9.0. Alacritty's `tty` module is the alternative if we want poller-integrated non-blocking I/O, but it is not separable from `alacritty_terminal`. |
| `wezterm-bidi` 0.2.3 | **Use as-is when bidi is in scope** | Published, `no_std`, MIT AND Unicode-DFS-2016. Last release 2024-01-27. If native text stacks (CoreText/DirectWrite) do bidi for us at render time, this may be unnecessary in the core; decide per design. |
| `wezterm-char-props` 0.1.3 | **Learn from; prefer maintained public crates** | Not on crates.io. Width tables are generated from `widecharwidth` for Unicode 16. For our core, `unicode-width` + a grapheme segmentation crate (plus emoji-presentation data) cover the same ground from crates.io; the *design* to copy is the Unicode-version/ambiguous-width switch and the VS15/VS16 width rule in `grapheme_column_width`. |
| `wezterm-blob-leases`, `wezterm-color-types`, `wezterm-dynamic` | **Only as transitive deps** | Published and MIT, but they exist to serve WezTerm's config/Lua layer and image spooling. |
| `mux`, `wezterm-gui`, `wezterm-font`, `alacritty` (GUI) | **Learn from only** | Unpublished application crates. Reference material for: reader + parser thread split and frame coalescing (`mux/src/lib.rs:142-330`), line quad/shape caches, glyph atlas strategies. Our renderers are native, so none of the GPU code applies. |

### Recommended shape

1. **Write our own state/grid crate**, borrowing: Alacritty's ring-buffer `Storage`, damage bounds and fair-lock read loop; WezTerm's grapheme-per-cell model, fat attributes, stable row index, per-line seqno, image-per-cell slicing and Unicode-version stack. Neither existing core satisfies "graphemes + images + flat FFI" at once: Alacritty's lacks the features, WezTerm's lacks the published, FFI-friendly shape.
2. **Reuse leaf crates as-is**: `portable-pty` (PTY), `vte` *or* `vtparse` (state machine), optionally `wezterm-bidi`.
3. **Decide once on the escape layer**: either vendor `wezterm-escape-parser` at a pinned commit (fastest route to sixel/kitty/OSC coverage, typed and testable) or build a typed layer over `vte::Perform` (smaller, faster, more work).
4. If time-to-first-working-terminal matters more than control, the fallback is a pinned git fork of `wezterm-term` behind our own FFI facade, with our own PTY threads and a sync-output timeout added. Treat it as owned code, not a dependency.

### Not verified

- Performance claims (vte vs vtparse throughput, ring vs deque scrolling) are structural readings of the code, not measurements.
- Internals of `Line::wrap`, `Row::append_front`, and the tail of `shrink_columns` were not read line by line.
- `crossfont` internals (Alacritty's rasteriser, latest 0.9.0 on crates.io, 2025-06-09) were not inspected; the "no shaping" statement rests on the absence of any shaping code or dependency in the Alacritty repo.
- Contents of the published `termwiz 0.23.3` tarball were not inspected.
- WezTerm's legacy key table in `termwiz/src/input.rs` was located (`encode` at line 241) but not audited for completeness.
