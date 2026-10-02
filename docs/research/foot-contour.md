# foot and Contour: source-level analysis for a Rust-core + native-UI terminal

Date checked: 2026-10-02. All citations are `path:line` in the repository at the commit below.
Anything not confirmed in the source is marked **unverified**.

| Project | Repository | Commit analysed | Commit date | Latest release |
| --- | --- | --- | --- | --- |
| foot | https://codeberg.org/dnkl/foot | `cb2771788998f6a6288e2de8273c55a8214551e0` | 2026-09-18 | 1.28.0 (git tag; `meson.build:2`) |
| Contour | https://github.com/contour-terminal/contour | `d8ce17bc34c653a3368d45f52bd7eea67452d673` | 2026-09-29 | 0.7.0 (tag `v0.7.0.8982`; `metainfo.xml:166`, dated 2026-08-17; 0.7.1 in development, `metainfo.xml:107`) |
| libunicode | https://github.com/contour-terminal/libunicode | `0cf7a3b227df23a51813dbfe7e92ff28255f1b91` | 2026-08-23 | v0.9.3 (git tag) |

Path prefixes: foot paths are relative to the foot repo root; Contour paths are relative to
`src/` unless they start with `docs/` or `metainfo.xml`; libunicode paths are relative to
`src/libunicode/`.

Important correction to common knowledge: Contour's "trivial line vs inflated line" storage no
longer exists as two buffer types. Lines are now a structure-of-arrays (`LineSoA`) with a blank
(unmaterialised) state and a cached `trivial` flag; `TrivialLineBuffer` survives only as a
deprecated compatibility view (`vtbackend/grid/Line.hpp:48-68`).

---

## Part 1. foot

### 1.1 PTY I/O and threading

- One main thread runs an epoll loop: `fdm.c:79` (`epoll_create1`), `fdm_add` `fdm.c:161`,
  `fdm_poll` `fdm.c:415` with `epoll_pwait` at `fdm.c:447`. PTY, Wayland socket, timers and the
  server socket are all fds on this loop.
- PTY read handler `fdm_ptmx` `terminal.c:244-389`: a 24 KiB stack buffer (`:273`), up to 10
  `read()` iterations per wake-up unless HUP (`:274-298`), each chunk goes straight to
  `vt_from_slave` (`:297`). There is no intermediate queue and no lock: the parser mutates the
  grid on the same thread that later renders it. Registered at `terminal.c:1513`.
- Coalescing of frames is done with two timerfds after each read ("delayed render"): a lower
  bound re-armed on every read and an upper bound that caps latency (`terminal.c:300-366`);
  defaults 0.5 ms and 8.3 ms (`doc/foot.ini.5.scd:1950-1973`).
- Back-pressure: PTY consumption is paused during an interactive resize (`terminal.c:263-271`,
  `term_ptmx_pause/resume` `terminal.c:392-405`).
- Writes to the child: `term_to_slave` `terminal.c:116` tries a direct write and queues the
  remainder, drained by `fdm_ptmx_out` `terminal.c:144`.
- Child spawn: `slave_spawn` `slave.c:374` (fork `:385`), `slave_exec` `slave.c:214`
  (`setsid` `:232`, `TIOCSCTTY` `:243`).
- The only extra threads are render workers: created at `terminal.c:709-744`
  (`thrd_create` `:738`), body `render_worker_thread` `render.c:2184-2300`. The main thread pushes
  dirty row numbers to a mutex-protected queue and waits on semaphores (`render.c:3557-3598`);
  sentinels `-1` frame done, `-2` exit, `-3` pre-apply old damage. Worker struct
  `terminal.h:694-711`; `workers` option `doc/foot.ini.5.scd:469-477`.

Design decision: single-threaded state, parallel only for the embarrassingly parallel step
(rasterising independent rows into a shared pixel buffer). No state lock exists at all.

### 1.2 Parsing

- State machine after the DEC ANSI parser (`vt.c:26` cites vt100.net); states `vt.c:28-54`,
  including explicit UTF-8 continuation states.
- `vt_from_slave` `vt.c:1101-1133` is a per-byte loop with a `switch` on the state that calls one
  `state_*_switch` function per state; those use GCC case ranges (ground: `vt.c:761-778`;
  "anywhere" transitions `vt.c:745-758`; 8-bit C1 is deliberately not supported, `vt.c:753-754`).
  This is a code-driven, not table-driven, state machine.
- CSI parameters: 16 params x 16 sub-params fixed arrays (`terminal.h:251-276`).
  `csi_dispatch` `csi.c:796`, SGR `csi_sgr` `csi.c:77`.
- OSC: bytes accumulated in a growable buffer (`action_osc_put` `vt.c:624`), dispatched at
  `action_osc_end` `vt.c:605-621` (buffer freed if it grew to >= 4096); UTF-8 bytes accepted
  (`vt.c:910-911`). `osc_dispatch` `osc.c:1273`, numbers handled at `osc.c:1301-1679`.
- DCS: hook/put/unhook function pointers (`terminal.h:297-303`), selected in `dcs.c:447-509`
  (sixel `q`, DECRQSS `$q`, BSU/ESU `=1s`/`=2s`, XTGETTCAP `+q`). Sixel gets a streaming `put`
  handler; the others buffer.
- Printing: `action_print` `vt.c:294` calls `term->ascii_printer`, a function pointer swapped
  between `ascii_printer_fast` (`terminal.c:4112-4151`) and `ascii_printer_generic`
  (`terminal.c:4106`) by `term_update_ascii_printer` (`terminal.c:4165-4183`). The fast variant
  is selected only when no "expensive" feature is active; the feature bits are listed in
  `terminal.h:407-418` (sixels present, OSC 8 open, underline style/colour, insert mode,
  non-ASCII charset).

Design decision: no bulk text scanner; instead the per-character path is made as short as
possible and the slow checks are hoisted out by swapping the function pointer when state changes.

### 1.3 Terminal state

- `struct terminal` `terminal.h:402+`. Cursor with explicit last-column flag `lcf`
  (`terminal.h:89-92`). Each grid carries its own cursor, saved cursor, saved attrs and the kitty
  keyboard flag stack of 8 (`terminal.h:220-249`).
- Charsets G0-G3 (`terminal.h:309-316`); DEC graphics translation inside `term_print`
  (`terminal.c:4013-4026`).
- Modes are plain bools in the struct (`terminal.h:435-449`); XTSAVE/XTRESTORE copy in a bitfield
  (`terminal.h:516-552`). DECSET/DECRST in `decset_decrst` `csi.c:321`; DECRQM `csi.c:635`.
- Tab stops: a linked list of column ints (`terminal.h:452`; set in `vt.c:444-458`; rebuilt on
  resize `render.c:4925-4927`).
- SGR state is the 8-byte `struct attributes` bitfield copied into each printed cell
  (`terminal.h:42-61`).

### 1.4 Screen buffer

- Cell: `struct cell { char32_t wc; struct attributes attrs; }`, 12 bytes, enforced by
  `static_assert` (`terminal.h:68-72`); rationale for the small cell in `terminal.h:35-41`.
  `attributes` packs 24-bit fg and bg, style bits, colour-source tags and the render bits
  `clean`, `selected`, `url` (`terminal.h:42-61`).
- Row: `struct row { cells, extra, dirty, linebreak, shell_integration }`
  (`terminal.h:148-160`). `extra` is a lazily allocated side structure for sparse data (OSC 8
  ranges, styled underline ranges; `terminal.h:103-146`).
- Grid: `struct grid` (`terminal.h:220-249`): `rows` is an array of row pointers, `num_rows` is a
  power of two, `offset` is the ring origin, `view` the viewport origin. Row lookup is
  `(offset + row) & (num_rows - 1)` (`grid.h:34-44`).
- Lazy rows: a row pointer is NULL until first touched (`grid_row_and_alloc` `grid.h:46-73`,
  `grid_row_alloc` `grid.c:438-456`), so configured scrollback costs one pointer per row until
  used.
- Wide characters: the head cell holds the code point, following cells hold
  `CELL_SPACER + remaining` (`terminal.h:63-66`, `terminal.c:3936`).
- Two grids, `normal` and `alt` (`terminal.h:424-426`). The switch for mode 1049 is at
  `csi.c:498-545`: selection cancelled, alt erased on entry, alt sixels destroyed on exit.

### 1.5 Scrollback

- Scrollback is the same ring as the screen. Row count is rounded up to a power of two:
  `1u << (32 - __builtin_clz(scrollback_lines + new_rows - 1))` (`render.c:4696-4705`); the alt
  grid is the power of two covering just the screen rows. Default `scrollback.lines` is 1000
  (`config.c:3571`).
- Scrolling is an offset bump: `term_scroll_partial` `terminal.c:3065-3129` adds to `offset`
  under the mask, swaps row pointers for non-scrolling regions, erases the rows that scroll in,
  calls `sixel_scroll_up`, adjusts selection and records scroll damage.
- No compression and no disk spill; memory is 12 bytes x columns per touched row.

### 1.6 Unicode

- Width: `c32width` maps to `utf8proc_charwidth` when built with utf8proc, else `wcwidth`
  (`char32.h:90-94`). utf8proc is an optional dependency (`meson.build:156`).
- Non-ASCII path `term_process_and_print_non_ascii` `terminal.c:4226-4445`:
  `utf8proc_grapheme_break_stateful` decides whether the code point extends the previous cell
  (`:4275`); first tries `fcft_precompose` to fold base+mark into one code point (`:4295-4323`);
  otherwise stores the cluster in the composed table.
- Composed table: `struct composed { chars, left, right, key, count, width, forced_width }`
  (`composed.h:6-14`), a binary search tree keyed by a hash of the code point sequence
  (`composed.c:23-39`, lookup `:55-67`, collision probing `:70-102`). The cell then stores
  `CELL_COMB_CHARS_LO + key` in `wc` (`terminal.h:63-66`). Limits: 255 code points per cluster
  (`terminal.c:4326-4341`) and a bounded key range (`terminal.c:4360`).
- Cluster width policy is configurable: max / double / wcswidth (`terminal.c:4388-4430`).
  VS15/VS16 force width 1/2 only for bases found in an `emoji_vs` table (`terminal.c:4397-4422`).
- DEC mode 2027 toggles `term->grapheme_shaping` (`csi.c:562-566`).
- Rendering a cluster: `fcft_rasterize_grapheme_utf32` (`render.c:950`), single code points
  `fcft_rasterize_char_utf32` (`render.c:974`). fcft's internal glyph cache: **unverified**
  (external library; `README.md:291-294` only says server mode shares "fonts and glyph cache").

Design decision: keep the cell at one `char32_t` and move rare multi-code-point clusters out of
line into an interned table; the interned key lives in a private range above Unicode.

### 1.7 Resize and reflow

- `grid_resize_and_reflow` `grid.c:816-1297`. A new row array is allocated and old rows are
  walked from the oldest scrollback row. Logical lines are reconstructed from the per-row
  `linebreak` flag (`grid.c:936-940`, `:1134-1173`); trailing empty lines are coalesced
  (`:987-997`, `:1171`).
- Tracking points: cursor, saved cursor, viewport origin plus caller-supplied points (selection
  start/end) are collected, sorted by position with `qsort` (`grid.c:862-887`) and translated as
  the copy cursor passes them (`grid.c:1063-1081`). This is one pass, O(cells + points log points).
- Wide characters that no longer fit at the end of a row are moved to the next row with spacers
  (`grid.c:1024-1030`), or replaced if wider than the whole grid (`grid.c:1089-1114`).
- Per-row ranges (OSC 8, underlines) are reflowed with `reflow_range_start/end`
  (`grid.c:641`, `:664`) and split at wraps (`grid.c:699-775`). Shell-integration marks are
  carried (`grid.c:1032`, `:1083-1087`). Old rows are freed as they are consumed (`grid.c:1175`).
- Sixels are re-anchored by row (`grid.c:909-919`) and destroyed if their row vanished
  (`grid.c:1281-1284`), then `sixel_reflow_grid` (`sixel.c:1045-1120`).
- Interactive resize is two-phase: while the user drags, foot shows a truncated copy of the
  viewport (`render.c:4781-4845`) with the PTY paused; the real reflow runs once afterwards in
  `delayed_reflow_of_normal_grid` (`render.c:4379-4433`, selection points `:4389-4401`) and then
  resumes the PTY.
- `grid_resize_without_reflow` exists (`grid.c:471`); that it is the alt-screen path is
  **unverified** (call site not read).

### 1.8 Input

- Legacy keys: `legacy_kbd_protocol` `input.c:1041`, static lookup tables in `keymap.h`.
- Kitty protocol: `kitty_kbd_protocol` `input.c:1221+` with a `bsearch` over `kitty-keymap.h`
  (`input.c:1251`); flags enum `terminal.h:207-218`; push `CSI > u` `csi.c:1743-1760` (stack of
  8, wraps when full), query `CSI ? u` `csi.c:1627-1635`; modifyOtherKeys `csi.c:1713`.
- Mouse: tracking and encoding enums `terminal.h:319-334` (UTF-8 encoding listed but not
  implemented), report in `report_mouse_click` `terminal.c:3417-3458`.
- Bracketed paste markers `selection.c:2453-2454`, `:2429-2430`; pastes use their own queue so
  they cannot interleave with key input (`terminal.h:502-504`).
- Focus events `\033[I` / `\033[O` `terminal.c:3342-3343`, `:3364-3365`.

### 1.9 OSC 8 hyperlinks

- Parsed in `osc_uri` `osc.c:479-536`: an explicit `id=` is hashed (sdbm), otherwise a random
  64-bit id is generated. `term_osc8_open/close` `terminal.c:4716-4736`.
- Storage is per row, not per cell: `row->extra` holds a sorted, non-overlapping array of
  `row_range { start, end, uri { id, char *uri } }` (`terminal.h:103-146`), maintained by
  `grid_row_range_put` (`grid.c:1337-1441`, extend/split/merge) and erased by
  `grid.c:1537-1598`. Each range owns its own `strdup` of the URI (`grid.c:1419`), so there is no
  global URI table and nothing to garbage-collect when rows scroll out.
- Cost: only rows that contain links allocate `extra`; the cell stays 12 bytes. The trade-off is
  duplicated URI strings for long multi-row links.

### 1.10 Sixel

- Parser state in `terminal.h:742-794`; `sixel_init` `sixel.c:66` returns the per-byte put
  handler; command handlers `sixel.c:1791-2030`; `sixel_unhook` `sixel.c:1132` finalises and
  trims transparent trailing rows.
- A finished image is a `struct sixel` (`terminal.h:162-205`): pixman image, size in cells,
  absolute grid row/col, the cell size at emission, and a cached scaled copy. Images are kept in
  a per-grid list sorted by end row (`sixel_insert` `sixel.c:389-414`); cells do not reference
  images.
- Scrolling: images whose rows leave the ring are destroyed (`sixel_scroll_up`
  `sixel.c:417-447`, `sixel_scroll_down` `:450-473`); since the position is an absolute ring row,
  scrolling otherwise costs nothing.
- Text over an image splits it into up to four remaining rectangles (`sixel_overwrite`
  `sixel.c:661+`; called from the print path at `terminal.c:4048`).
- Reflow: `sixel_reflow_grid` `sixel.c:1045-1120` drops images that cross the scrollback wrap or
  no longer fit and re-inserts the rest. Font size change rescales lazily
  (`sixel_cell_size_changed` `sixel.c:952`, `sixel_sync_cache` `:962`).
- Drawn before text rows by `render_sixel_images` (`render.c:1686`, called `render.c:3554`).
- No kitty graphics protocol in foot (no handler in `dcs.c:447-509`; APC handling
  **unverified**).

### 1.11 Rendering and damage

- Two-level dirty state: per-row `dirty` and per-cell `attrs.clean` (early-out in `render_cell`
  `render.c:701-704`; `term_damage_rows` `terminal.c:2489-2498`).
- Scroll damage is recorded separately as a coalesced list (`term_damage_scroll`
  `terminal.c:2628-2653`, `struct damage` `terminal.h:94-101`) and replayed on the pixel buffer
  as an SHM scroll or `memmove` (`grid_render_scroll` `render.c:1319-1421`; only used when fewer
  than half the rows scrolled, `:1377-1381`).
- Buffer-age handling: when the compositor hands back an older buffer, the previous frame's
  damage is re-applied (`render.c:3198+`), optionally on a worker thread ahead of time
  (`render.c:2246-2293`, `:3374-3387`).
- Frame pacing: a frame callback is requested per commit (`render.c:3678-3680`,
  `frame_callback` `render.c:4313-4358`). `fdm_hook_refresh_pending_terminals`
  (`render.c:5146-5205`) renders immediately if no callback is outstanding, otherwise just marks
  the terminal pending.
- Synchronized output (2026): `term_enable_app_sync_updates` `terminal.c:3834-3862` suppresses
  grid rendering (`render.c:5165`) with a 1 s safety timer; disable at `terminal.c:3865-3877`.
- Damage is reported to the compositor per region (`render.c:3605`, `:1419`).
- Selection highlight bits are synced into cells at render time (`render.c:3431-3448`).

### 1.12 Performance techniques

- 12-byte cells with out-of-line rare data (1.4, 1.6, 1.9).
- Power-of-two ring + lazy rows; scroll is pointer arithmetic (1.4, 1.5).
- ASCII printer function-pointer swap (1.2).
- Row-parallel CPU rasterisation with per-worker damage (1.1).
- Scroll-as-memmove on the pixel buffer and buffer-age reuse (1.11).
- Delayed-render timers so a burst of PTY output renders once (1.1).
- Server mode: one process, many windows, shared fonts (`README.md:278-302`; `server.c:438`,
  `:144`). PGO builds are documented and scripted (`INSTALL.md:221-367`, `pgo/`).

---

## Part 2. Contour

### 2.1 PTY I/O and threading

- `vtpty::Pty` is an abstract interface (`vtpty/Pty.hpp:91-153`): `start`, `read(BufferObject&,
  timeout, size)` (`:130`), `wakeupReader` (`:140`), `write` (`:147`), `resizeScreen` (`:153`).
  Implementations: `UnixPty` (`openpty` `vtpty/UnixPty.cpp:93`, non-blocking master `:232`, a
  read selector with a break-pipe wake-up `:289-291`, and an extra "stdout fast pipe" `:237-241`,
  `TIOCSWINSZ` `:463`), `ConPty` (`CreatePseudoConsole` loaded dynamically
  `vtpty/ConPty.cpp:76-99`, blocking `ReadFile` `:249`; `wakeupReader` has an open question in
  the source at `:258-261`), plus SSH, channel and mock PTYs.
- Each session has a dedicated reader thread `Terminal.Loop`
  (`contour/session/TerminalSession.cpp:477`, loop `:513-532`) calling
  `Terminal::processInputOnce` (`vtbackend/screen/Terminal.cpp:365-457`): read into a pooled
  buffer (`readFromPty` `:323-355`; pool objects 1 MiB, read size 4096,
  `vtbackend/screen/Settings.hpp:180`, `:185`), take the state lock, `parseFragment` (`:433`),
  release, then `screenUpdated()` outside the lock (`:449-450`).
- One plain `std::mutex` `_stateMutex` guards all terminal state
  (`vtbackend/screen/Terminal.hpp:2402-2408`, `lock/unlock` `:1341-1342`); cross-thread flags are
  atomics (`:2617-2644`). The parser thread holds the lock for a whole output burst
  (`docs/internals/software-architecture/index.md:177-187`).
- The render thread never reads the grid; it reads a snapshot (2.11).

### 2.2 Parsing

- Table-driven: `ParserTable::get()` is `constexpr` and indexed `transitions[state][byte]` with
  separate entry/exit/event tables (`vtparser/Parser-impl.hpp:490-548`). States and actions:
  `vtparser/Parser.hpp:24`, `:201`, `:214`.
- The parser is a template over its event listener (`Parser<EventListener, TraceStateChanges>`
  `vtparser/Parser.hpp:681-796`), constrained by `ParserEventsConcept`
  (`vtparser/Parser.hpp:500-535`), so dispatch is static.
- Bulk fast paths chosen per state in `parseFragment` (`Parser-impl.hpp:383-413`):
  - Ground: `parseBulkText` (`Parser-impl.hpp:600-697`) calls libunicode
    `scan_text(state, chunk, maxColumns)` (`:634`), which returns how many bytes form printable
    text fitting in the remaining columns, then emits one `print(string_view, cellCount)`
    (`:660`). Trailing C0 controls are executed inline (`:669-693`; the source comment claims
    about 50x for cat-like workloads).
  - DCS pass-through and APC strings have their own bulk scanners
    (`vtparser/Parser.hpp:751`, `:763`).
- The bulk path is offered only when the terminal says it can take it:
  `Terminal::maxBulkTextSequenceWidth` returns 0 unless on the primary screen with a trivial
  current line (`vtbackend/screen/Terminal.cpp:1711-1731`).
- `scan_text` is SIMD: ASCII run detection dispatches to 512/256/128-bit variants at runtime
  (libunicode `scan.cpp:65-78`; high-bit mask test `scan_simd_impl.h:36`, `:74`); the non-ASCII
  part decodes UTF-8 and accumulates cluster widths (`scan.cpp:81+`, `:165-191`).
- DCS payloads go to `ParserExtension` objects hooked by the screen
  (`vtbackend/screen/Screen.cpp:7266-7272`: sixel, ReGIS, STP, DECRQSS, DECUDK, XTGETTCAP, GIP).
  Sequences are dispatched in `Screen::processSequence` (`Screen.cpp:5666`) via the function
  table in `vtbackend/vt/Functions.hpp`.

### 2.3 Terminal state

- `struct Cursor { position, originMode, wrapPending, graphicsRendition, charsets, hyperlink }`
  (`vtbackend/screen/Cursor.hpp:15-26`).
- Modes: `class Modes` with bitsets for ANSI and DEC modes, a "frozen" bitset and a save stack
  per mode (`vtbackend/screen/Terminal.hpp:103-168`); number mapping in
  `vtbackend/core/Primitives.hpp:1379-1419`.
- Tab stops: `std::vector<ColumnOffset> _tabs` (`Terminal.hpp:2735`).
- Screens are "pages": primary is page 0, alternate is a fixed page index
  (`Terminal.hpp:1445-1446`, `:1528-1531`), plus two one-line status screens
  (`Terminal.hpp:2599-2600`). Alt-screen variants 47/1047/1049 are described declaratively as
  `AlternateScreenBehavior { carryCursor, clearOnEnter, clearOnExit }`
  (`Primitives.hpp:1104-1139`).

### 2.4 Screen buffer

- `LineSoA` (`vtbackend/grid/LineSoA.hpp:55-138`): parallel 64-byte-aligned arrays grouped by
  access temperature: `codepoints`, `widths`, `scales` (hot); `sgr` (warm; `GraphicsAttributes`
  is three colours plus flags, `vtbackend/core/GraphicsAttributes.hpp:14-27`); `hyperlinks`,
  `textScaleExtras` (cold); `clusterSize`, `clusterPoolIndex` and a per-line `clusterPool` for
  the extra code points of multi-code-point cells; an optional map column -> image fragment.
  Maximum cluster size 16 (`LineSoA.hpp:37`).
- Blank state: all arrays empty; construct/reset is O(1) (`LineSoA.hpp:183`, `:187-190`). A line
  is materialised on first write (`materializedStorage()` used at `Screen.cpp:474`).
- `trivial` flag (`LineSoA.hpp:118-132`): true while the line has uniform SGR and hyperlink, no
  wide continuation, no image, no multi-code-point cluster. It gates both the bulk write path
  and the per-line render path.
- `class Line` wraps the SoA plus flags Wrappable/Wrapped/Marked
  (`vtbackend/grid/Line.hpp:73+`, `:336-355`).
- Bulk ASCII write: `Screen::writeText(string_view, cellCount)` (`Screen.cpp:444-528`) checks
  pure ASCII, no insert mode, full margins, US-ASCII charset, then `writeTextToSoA` fills the
  arrays directly (`:486-491`). Everything else goes per code point (`:530-619`).

### 2.5 Scrollback

- `using Lines = crispy::Ring<Line>` (`vtbackend/grid/Grid.hpp:126`); one ring holds history and
  page, line offset 0 is the top of the page and -1 the newest history line
  (`Primitives.hpp:139-143`). `Ring` is a `std::vector` with a moving zero index
  (`crispy/Ring.hpp:149`).
- `Grid::scrollUp` (`vtbackend/grid/Grid.cpp:438-498`): infinite history appends lines; at
  capacity the ring rotates and the recycled line is reset to blank; otherwise it appends.
- Limits: `HistoryLimits { guaranteed, capacity }` (`Primitives.hpp:116-137`) with an `Infinite`
  alternative (`Grid.cpp:450`); the headroom between the two allows eviction to snap to shell
  command boundaries. Config default 1000 lines (`contour/config/Config.hpp:305`).
- Stable row ids and per-line generations exist for delta sync
  (`Grid.hpp:795-840`, `forEachLineChangedSince` `Grid.hpp:859`).

### 2.6 Unicode

- libunicode supplies: multistage property tables (`codepoint_properties.h:80-96`) with
  `char_width`, East Asian Width and emoji flags (`:28-67`); a stateful grapheme segmenter
  (`grapheme_segmenter.h:29-54`, with GB9c and GB11 state); and one width authority,
  `grapheme_cluster_width_accumulator` (`width.h:33-60`), used by both `scan_text` and
  `grapheme_cluster_width` so the scanner and the grid cannot disagree (`width.h:24-29`).
- VS16 widens only a base that has a defined emoji variation sequence and is one column wide;
  VS15 narrows (`width.cpp:76-99`). Skin-tone modifiers and ZWJ handling at `width.cpp:114-116`,
  `width.h:59`.
- Grid side: a code point that does not break is appended to the previous cell
  (`Screen.cpp:605-613`, `:751-759`); width changes after the first code point are applied by
  `applyClusterWidthChange` (`Screen.cpp:841+`).
- The grapheme state is rebuilt from the previous cell's stored code points instead of being
  carried in the parser (`Screen.cpp:536-563`, `:716-744`). The source explains this was needed
  because the bulk scanner and the per-code-point path would otherwise double-process state.
- Mode 2027 (`Primitives.hpp:901-902`, `:1419`; spec at
  https://github.com/contour-terminal/terminal-unicode-core, referenced from
  `docs/vt-extensions/unicode-core.md`, checked 2026-10-02): Contour always clusters; the mode
  only selects `ClusterAware` vs `FirstCodepoint` width policy (`Screen.cpp:828-839`,
  `LineSoA.hpp:243-255`).

### 2.7 Resize and reflow

- `Grid::resize` (`vtbackend/grid/Grid.cpp:783-1180`). Growing columns skips work when no used
  line is wrapped (`:864`). Shrinking columns (`:989-1136`) has a fast path when nothing needs
  cutting; otherwise it rebuilds the line container, carrying each line's overflow
  (`Line::reflow` `vtbackend/grid/Line.cpp:15-60`, which trims trailing blanks and returns the
  overflow columns only for wrappable lines) into the following wrapped line or new wrapped
  lines.
- Reflow can be disabled per line range by the application (DEC mode 2028,
  `Primitives.hpp:904-909`, `docs/vt-extensions/line-reflow-mode.md`).
- Weak point: the column-shrink path ends with `return cursor; // TODO` (`Grid.cpp:1134`), i.e.
  the cursor is not tracked through that reflow. There is no generic tracking-point mechanism.
- Selection is cleared on resize rather than remapped (`Terminal.cpp:2151-2153`).
- Only the active buffer is resized; others are resized when switched to
  (`Terminal.cpp:2178-2179`).
- A column change bumps the stable-id generation, invalidating remote deltas
  (`Grid.hpp:814-815`).
- Whether image fragments are carried across a column reflow: **unverified**.

### 2.8 Input

- All encoding lives in the core: `InputGenerator` (`vtbackend/input/InputGenerator.hpp:602+`)
  accumulates bytes in a pending buffer that the owner drains with `peek`/`consume`
  (`:731-743`).
- Keyboard: `StandardKeyboardInputGenerator` (`:432`) and `ExtendedKeyboardInputGenerator`
  (`:535`) for the kitty/CSI u protocol with the five flags (`:522-530`) and a flag stack of 32
  (`:538`, `:598`). Sequences `CSIUENTER/QUERY/ENHCE/LEAVE` handled at
  `Screen.cpp:7165-7189`. A Win32 input mode generator also exists (`:790`).
- Key events carry press/repeat/release (`:390-395`) and a `KeyIdentity` (`:406`), so the UI
  must supply physical-key information.
- Mouse: protocols 9/1000/1001/1002/1003 (`:26-38`), transports Default/Extended/SGR/SGRPixels/
  URXVT (`:374-387`), encoders at `InputGenerator.cpp:1306-1415`.
- Bracketed paste `InputGenerator.cpp:1104-1117`; focus `:1145-1160`.
- Frontend entry points: `sendKeyEvent` `Terminal.cpp:1005`, `sendMousePressEvent` `:1108`,
  `sendMouseMoveEvent` `:1420`, `sendMouseReleaseEvent` `:1491`, `sendFocusInEvent` `:1536`,
  `sendFocusOutEvent` `:1550`, `sendPaste` `:1564`.

### 2.9 OSC 8 hyperlinks

- Per-cell 16-bit id (`HyperlinkId` `vtbackend/core/Hyperlink.hpp:45`) stored in the cold
  `LineSoA::hyperlinks` array; the URI lives in a terminal-wide
  `LRUCache<HyperlinkId, shared_ptr<HyperlinkInfo>>` of 1024 entries (`Hyperlink.hpp:47-82`).
- `Screen::hyperlink` (`Screen.cpp:2126-2150`) deduplicates by `id + uri` through a linear scan
  (`Hyperlink.hpp:70-81`) and otherwise allocates the next id. The source carries a TODO about
  eviction (`Screen.cpp:2146-2149`): cells can outlive their cache entry, and the 16-bit id can
  wrap.

### 2.10 Images

- Three layers (`vtbackend/core/Image.hpp`): `Image` (pixels, 32-bit id, `:102-141`),
  `RasterizedImage` (an image fitted to a cell rectangle with alignment/resize policy,
  `:208-281`), `ImageFragment` (one cell's slice: shared pointer plus cell offset, `:290-321`).
- Attachment: every covered cell gets an `ImageFragment` in the line's fragment map
  (`Screen::renderImage` `Screen.cpp:2781+`, loop at about `:2846-2853`;
  `vtbackend/grid/CellProxy.hpp:328-331`). Because fragments live in lines, images scroll into
  history with the text for free and die by reference counting when the last cell is overwritten
  or evicted (`Image::~Image` calls the pool's removal callback, `vtbackend/core/Image.cpp:38-42`,
  which reaches the frontend as `Events::discardImage`, `Terminal.cpp:4630-4633`).
- `ImagePool` (`Image.hpp:361-413`): creates images, keeps a name -> image LRU for named uploads
  and a weak id index.
- Protocols: Sixel (`SixelParser` + `SixelImageBuilder`,
  `vtbackend/graphics/SixelParser.hpp:97`, `:449`; entry `Screen::sixelImage` `Screen.cpp:2746`),
  the "Good Image Protocol" DCS with operations upload/render/release/oneshot/query
  (`Screen.cpp:7777-7797`), and kitty graphics over APC (`Screen.cpp:5372-5557`). Size ceiling
  default 800x600 (`vtbackend/screen/Settings.hpp:53`).
- Cost of this design: a large image creates one heap fragment per cell and makes every covered
  line non-trivial.

### 2.11 Rendering

- Snapshot types (`vtbackend/render/RenderBuffer.hpp`): `RenderCell` (code points, image
  fragment, position, resolved RGB attributes, width, `groupStart`/`groupEnd`; `:62-76`),
  `RenderLine` (whole-line text with one attribute set; `:81-90`), `RenderCursor` (`:105-114`),
  `RenderBuffer { cells, lines, gutter, cursor, frameID }` (`:116-131`).
- Producer: `Grid::render` walks visible lines; blank or trivial lines emit one `RenderLine`,
  others emit per-cell `RenderCell`s (`vtbackend/grid/Grid.hpp:1264-1300`), through
  `RenderBufferBuilder` (`vtbackend/screen/RenderBufferBuilder.hpp:20-70`), which resolves
  palette, reverse video, selection, search matches, cursor and IME preedit into final colours.
- Handoff: `RenderDoubleBuffer` holds two buffers, an atomic back index and a small state machine
  (`RenderBuffer.hpp:153-193`). `Terminal::ensureFreshRenderBuffer`
  (`Terminal.cpp:479-530`) fills the back buffer under the state lock and swaps;
  `swapBuffers` uses `try_lock` on the reader lock so the terminal thread never waits for the
  renderer (`vtbackend/render/RenderBuffer.cpp:10-33`). The renderer takes
  `Terminal::renderBuffer()` (`Terminal.hpp:1332`), a lock-guard handle on the front buffer.
- There is no cell-level damage tracking: each refresh rebuilds the whole visible page, throttled
  by the refresh rate (default 30 Hz, `Settings.hpp:119`). Synchronized output (2026) suppresses
  refresh with a 150 ms timeout (`Terminal.cpp:4661-4670`, `Settings.hpp:118`).
- Consumer: `vtrasterizer::Renderer::render` (`vtrasterizer/Renderer.cpp:658`, `:759-763`,
  `:882-922`) feeds cells and lines to background, decoration, text and image renderers, which
  emit tile draw commands to an abstract `RenderTarget` (`vtrasterizer/RenderTarget.hpp:92-149`).
- Text: runs are grouped (`groupStart`/`groupEnd`), shaped once and cached by a hash of text +
  style in an LRU (`vtrasterizer/TextRenderer.cpp:189`, `:279`, `:439-441`, `:645-647`); glyphs
  are cached as atlas tiles in a fixed-capacity LRU hashtable
  (`vtrasterizer/TextureAtlas.hpp:248`, `:329`), with direct-mapped slots for ASCII that bypass
  the LRU (`TextureAtlas.hpp:306-316`, `TextRenderer.cpp:380-390`). The atlas must be at least
  3x the page cell count or tiles get recycled within a frame
  (`vtrasterizer/AtlasBudget.hpp:14-47`).
- Shaping backends behind `text::Shaper` (`text_shaper/Shaper.hpp:88-170`): HarfBuzz/FreeType
  (`OpenShaper.cpp`) and DirectWrite, with CoreText/Fontconfig/DirectWrite font locators.

### 2.12 Performance techniques

- SIMD text scan with a column budget, one event per text run (2.2).
- Bulk ASCII write straight into SoA arrays (2.4).
- Blank lines cost nothing; trivial lines render as one object (2.4, 2.11).
- Parser as a constexpr table with statically dispatched listener (2.2).
- Double-buffered snapshot with non-blocking swap (2.11).
- Pooled 1 MiB read buffers; the parser hands `string_view`s into them (2.1).
- Word-level shaping cache and LRU glyph atlas with direct-mapped ASCII (2.11).

---

## Part 3 (a). Contour's library split and how it maps to "core computes, native UI renders"

Dependency facts:

- `vtparser` links only GSL, tracy and libunicode (`vtparser/CMakeLists.txt:10-14`).
- `vtbackend` links GSL, boxed-cpp, threads, tracy, core, crispy, libunicode, `vtparser`,
  `vtpty` (`vtbackend/CMakeLists.txt:161-172`). No Qt, no GPU, no font library.
- `vtrasterizer` links `vtbackend` and `text_shaper` (`vtrasterizer/CMakeLists.txt:60`) and draws
  only through the abstract `RenderTarget`.
- The Qt application sits on top; a Qt-free daemon (`vthost`) owns `{Pty, Terminal}` pairs and
  serves either raw bytes or per-line grid deltas to thin clients
  (`docs/internals/vthost.md:1-40`). This is direct evidence the core runs headless.

The frontend contract of `vtbackend`:

1. Core -> UI callbacks: `Terminal::Events` (`vtbackend/screen/Terminal.hpp:299-417`): bell,
   `screenUpdated`, `renderBufferUpdated`, clipboard copy/paste request, title, window
   resize/move/fullscreen requests, pointer shape, notifications, progress, font get/set,
   `discardImage`, input-mode and scroll-offset changes, `onClosed`. Some fire on the parser
   thread with the state lock held (`Terminal.hpp:328-336`, `:412-416`).
2. UI -> core calls: `sendKeyEvent`, `sendMouse*Event`, `sendFocus*Event`, `sendPaste`,
   `resizeScreen`, `tick` (line references in 2.8; `resizeScreen` `Terminal.cpp:2156`, `tick`
   `Terminal.hpp:1281`).
3. Frame data: `RenderBuffer` obtained through `renderBuffer()` (2.11).

Fit for an FFI boundary:

- Good: the snapshot is the only thing the renderer touches, it is plain data with colours
  already resolved, and the producer never blocks on the consumer. Input encoding is entirely in
  the core, so each native UI only translates platform events into a neutral key/mouse struct.
- Needs change before copying:
  - `RenderCell` owns a `std::u32string` and a `shared_ptr`; across FFI this should be flat
    arrays (code point pool + offsets, image handle ids).
  - The snapshot is a full page every time. A native UI benefits from per-line generations so
    it can skip unchanged lines; Contour already has this for the daemon protocol
    (`Grid.hpp:795-859`) but not in `RenderBuffer`.
  - Callbacks fired under the state lock are a deadlock hazard when the UI calls back into the
    core; across FFI they should be queued events drained by the UI thread.
  - `Events` mixes rendering notifications with window-management requests; splitting them keeps
    the FFI surface small.
- Contour stops at the snapshot and then shares one rasteriser across platforms. A SwiftUI/WinUI
  product would stop at the same line but replace `vtrasterizer` + `text_shaper` with CoreText/
  Metal and DirectWrite/Direct3D. The `RenderLine` (uniform-attribute run) vs `RenderCell` split
  maps well to native text APIs, which prefer whole runs.

---

## Part 3 (b). Recommendations

### Borrow from foot

- Small fixed cell with rare data out of line (hyperlinks and styled underlines as per-row
  ranges, clusters in an interned table). Reason: memory per scrollback row dominates; per-row
  ranges also remove the need for a global link table and its eviction problem (compare 1.9 with
  Contour's TODO in 2.9).
- Power-of-two ring of lazily allocated rows shared by screen and scrollback. Reason: scroll is
  O(1) and unused scrollback is nearly free.
- Tracking-point reflow in a single pass (cursor, saved cursor, viewport, selection). Reason: it
  is the one design here that keeps all anchors correct; Contour's shrink path does not track
  even the cursor.
- Two-phase interactive resize with PTY paused and one final reflow. Reason: avoids reflowing
  the whole scrollback on every drag step.
- Row dirty flags plus separate scroll damage, exported with the frame. Reason: lets a native
  renderer redraw only changed rows.
- Swapping the print routine when "expensive" state toggles. Reason: keeps the common path free
  of feature checks; in Rust this is an enum or function pointer chosen on state change.
- Images as a separate positioned list rather than per-cell fragments. Reason: no per-cell
  allocation and lines stay simple; cost is explicit overwrite/split and scroll-out handling.
- Render coalescing with a short lower and a capped upper delay, and a hard timeout on
  synchronized output.

### Borrow from Contour

- Library layering: parser, PTY, backend, rasteriser as separate crates with the backend free of
  UI and font dependencies; an abstract PTY trait including a wake-up call.
- Bulk text scanning with a column budget (SIMD ASCII detection, one event per run) and a direct
  write into line storage when the line is simple.
- Blank/trivial line states. Reason: most lines are uniform; both writing and snapshotting them
  become O(1) or one memcpy, and a uniform run is exactly what CoreText/DirectWrite want.
- One width authority shared by scanner and grid (the accumulator in libunicode). Reason: the
  class of bug where the scanner and the grid disagree on a cluster's width is otherwise easy to
  reintroduce.
- Mode 2027 semantics and always-on clustering.
- Snapshot handoff: core fills a back buffer under its lock, swaps without waiting for the
  reader, UI reads the front buffer. This is the natural FFI frame API.
- Input encoding in the core behind a neutral key event with press/repeat/release and physical
  key identity; kitty flag stack per screen.
- Stable row ids and per-line generations for incremental sync.
- Declarative alt-screen behaviour table for 47/1047/1049.

### Avoid

- Callbacks into the frontend while holding the state lock (Contour). Use an event queue.
- A single coarse state mutex held for a whole parse burst if the UI thread ever needs state
  synchronously (Contour); either keep the UI on snapshots only, or bound the burst.
- Full-page snapshot rebuild on every refresh with no damage information (Contour).
- Owning containers and shared pointers inside per-cell snapshot records (Contour); they do not
  cross FFI and they allocate per cell.
- A bounded LRU for hyperlink data referenced by ids stored in scrollback, and 16-bit ids that
  can wrap (Contour).
- Per-cell image fragments (Contour): allocation per covered cell and loss of the trivial-line
  fast path.
- Clearing the selection and not tracking the cursor on reflow (Contour).
- Rebuilding grapheme state from the previous cell on every non-ASCII code point because two
  write paths disagree about who owns the segmenter state (Contour); give the state one owner.
- Single-threaded parse + render on the UI thread (foot). It is correct for a Wayland client but
  a SwiftUI/WinUI main thread must not block on PTY bursts; keep foot's data structures, not its
  threading.
- CPU row rasterisation into shared memory and SHM scroll tricks (foot); they are Wayland
  specific and replaced by the platform GPU text stack.
- Code-driven per-byte state functions relying on GCC case ranges and no bulk path (foot);
  a table or `match` plus a bulk scanner is both portable and faster on plain text.
- Hard limits baked into the protocol layer without reporting (foot: 16x16 CSI params, 255 code
  points per cluster; Contour: 16 code points per cluster) are acceptable, but they must be
  explicit and tested.
- Global linked list for tab stops (foot); a bitset per column is simpler.

---

## Unverified items (collected)

- fcft's glyph cache internals (external library, not in the foot tree).
- Call site that routes the alt grid to `grid_resize_without_reflow` in foot.
- Whether foot ignores APC entirely (only DCS handlers were read).
- Whether Contour carries image fragments across a column reflow.
- Performance figures quoted from source comments (for example "about 50x") were not measured.
- `docs/internals/software-architecture/index.md:195` says the display "typically" uses OpenGL
  while the tree contains an RHI renderer; the actual default backend was not checked.
