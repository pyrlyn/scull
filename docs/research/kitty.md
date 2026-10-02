# kitty: bytes-to-pixels pipeline (source analysis)

- Repository: https://github.com/kovidgoyal/kitty
- Analysed commit: `852fef34534b232b4496be2d0fe09586bfa44bc1` (HEAD of default branch, commit date 2026-10-02, shallow clone)
- Latest release: **0.49.2**, dated 2026-10-01 (`docs/changelog.rst:239`; `kitty/constants.py:25` has `Version(0, 49, 2)`). `0.50.0 [future]` is the unreleased section (`docs/changelog.rst:202`).
- Date checked: 2026-10-02.

Citation convention: `path:line` refers to the commit above. Protocol specs were read from `docs/*.rst` in the repo at that commit; these files are the source of the pages under `https://sw.kovidgoyal.net/kitty/`. The rendered website itself was **not** fetched, so the URL mapping (e.g. `docs/keyboard-protocol.rst` -> `https://sw.kovidgoyal.net/kitty/keyboard-protocol/`) is by convention and the published site may lag or lead the commit.

Anything marked **unverified** was not confirmed in the source.

---

## 1. PTY I/O and threading

Files: `kitty/child-monitor.c`, `kitty/vt-parser.c`, `kitty/loop-utils.c`.

Threads:

| Thread | Name | Entry | Job |
| --- | --- | --- | --- |
| I/O | `KittyChildMon` | `io_loop()` `child-monitor.c:1809-1818` | `poll()` over all child PTY fds, read bytes, write queued bytes, reap children |
| Peer/remote control | `KittyPeerMon` | `talk_loop()` `child-monitor.c:2205-2209` | remote-control sockets |
| Main (UI) | — | `process_global_state()` `child-monitor.c:1563-1596` | parse, mutate screen state, render |

Key facts:

- One I/O thread serves all windows: static arrays `children[MAX_CHILDREN]` and `children_fds[MAX_CHILDREN + EXTRA_FDS]` (`child-monitor.c:84-88`), `MAX_CHILDREN` is 512 (`data-types.h:157`). A single `poll()` covers a wakeup fd, a signal fd and every PTY (`child-monitor.c:1836-1844`).
- **The I/O thread only moves bytes; it never parses.** `read_bytes()` asks the parser for a writable region of its buffer, `read()`s straight into it and commits (`child-monitor.c:1673-1693`). No intermediate copy.
- **Parsing runs on the main thread**: `process_global_state()` calls `parse_input()` then `render()` (`child-monitor.c:1576-1577`); `do_parse()` calls `self->parse_func(screen, &pd, flush)` (`child-monitor.c:532-541`). So screen state is only ever mutated by the main thread and needs no lock for rendering.
- The shared object is the parser's byte buffer, guarded by one mutex per parser (`PS.lock`, `vt-parser.c:241`). Layout: one fixed 1 MiB buffer (`BUF_SZ`, `vt-parser.c:18`) with a `read {consumed,pos,sz}` cursor for the parser and a `write {offset,sz,pending}` cursor for the I/O thread (`vt-parser.c:243-249`).
  - Writer: `vt_parser_create_write_buffer()` / `vt_parser_commit_write()` (`vt-parser.c:1625-1657`).
  - Reader: `run_worker()` takes the lock only to absorb `write.pending` into `read.sz`, then **parses without the lock**, re-locks, and loops while more data arrived (`vt-parser.c:1583-1621`; the comment at `1602-1604` states the invariant: the writer only touches `write.pending` and space beyond `read.sz + write.pending`).
- **Back-pressure**: the I/O thread only sets `POLLIN` for a child when `vt_parser_has_space_for_input()` is true (`child-monitor.c:1832`, `vt-parser.c:1659-1665`). A full buffer stops reading, so the kernel PTY buffer blocks the child. When the parser frees space it sets `write_space_created` and the main thread wakes the I/O loop (`vt-parser.c:1612`, `child-monitor.c:537`).
- **Latency/throughput coalescing** (`input_delay`, default 3 ms, `kitty/options/definition.py:1478-1479`; `repaint_delay`, default 10 ms, `definition.py:1463-1464`):
  - The I/O thread wakes the main loop at most once per `input_delay`, except that "small" pending input (< 1024 bytes, `SMALL_PENDING_INPUT_THRESHOLD`, `vt-parser.c:23-25`) wakes immediately once, so typed-character echo is not delayed (`child-monitor.c:1907-1919`).
  - The parser itself declines to parse until `input_delay` elapsed unless flushing, input is small, or the buffer is nearly full (`vt-parser.c:1594`).
  - `render()` skips a frame if less than `repaint_delay` passed and nothing was read (`child-monitor.c:1140-1142`).
- Writes to the child go through a per-screen `write_buf` under `write_buf_lock` (`screen.h:152-154`); the I/O thread drains it on `POLLOUT` (`write_to_child()`, `child-monitor.c:1774-1807`). There is a 100 MiB cap (`child-monitor.c:346`).
- Design statement in docs: "Interaction with child programs takes place in a separate thread from rendering" (`docs/performance.rst:8-9`).

Design decision: I/O thread = dumb byte pump with back-pressure; parse + state + render all on one thread. This avoids a lock around the grid at the price of parsing competing with rendering on the UI thread.

## 2. Escape-sequence parsing (`kitty/vt-parser.c`)

- **State machine with 8 states**: `VTE_NORMAL, VTE_ESC, VTE_CSI, VTE_OSC, VTE_DCS, VTE_APC, VTE_PM, VTE_SOS` (`vt-parser.c:188-197`). Top-level switch in `consume_input()` (`vt-parser.c:1515-1549`). Only 7-bit `ESC`-introduced sequences change state: normal mode decodes "to ESC" (see below), and `consume_esc()` selects the sub-state from the byte after ESC (`vt-parser.c:327-340`). 8-bit C1 introducers are not a state trigger in this code path.
- **Normal mode = SIMD UTF-8 decode until ESC**: `consume_normal()` calls `utf8_decode_to_esc()` on the raw buffer, gets a `uint32_t` code point array, and hands the whole run to `screen_draw_text()` in one call (`vt-parser.c:276-291`). Control bytes other than ESC travel inside the decoded run and are handled in the draw loop (`draw_control_char`, `screen.c:1293`, called at `screen.c:1346-1347`).
- **SIMD dispatch**: implementations at 128/256/512-bit plus scalar (`simd-string.h:113-126`), selected at startup by CPU probe (`__builtin_cpu_supports` for sse4.2 / avx2 / avx512, `simd-string.c:353-356`), overridable with `KITTY_SIMD` (`simd-string.c:385-389`). ARM uses the same code through SIMDe ("simde takes care of NEON on Apple Silicon", `simd-string.c:361`). The decoder has an all-ASCII fast path that checks two vectors at once for "ESC or non-ASCII" (`simd-string-impl.h:686-696`). The buffer is over-allocated by 64 bytes so wide loads never read past the end (`BUF_EXTRA`, `vt-parser.c:19-20`, `230`).
- **String sequences (OSC/DCS/APC/PM/SOS)**: `accumulate_st_terminated_esc_code()` scans for the terminator with SIMD `find_either_of_two_bytes(BEL, ESC_ST)` (`vt-parser.c:404-427`, `451-480`), null-terminates **in place** and dispatches a pointer into the read buffer: zero-copy payloads.
  - Max length `MAX_ESCAPE_CODE_LENGTH = BUF_SZ/4` = 256 KiB (`vt-parser.c:21`); longer codes are dropped with an error (`vt-parser.c:463-477`). OSC 52 is special-cased: it is dispatched in partial chunks so clipboard payloads can exceed the limit (`vt-parser.c:435-448`, `464-475`).
- **CSI**: `ParsedCSI` holds primary/secondary/trailer bytes, up to 256 int params (`MAX_CSI_PARAMS`, `vt-parser.c:22`) and a parallel `is_sub_param[]` array for colon sub-parameters (`vt-parser.c:218-227`, `925-932`). Digits are capped at 16 (`vt-parser.c:834`). Control characters embedded inside a CSI are executed immediately (`CSI_NORMAL_MODE_EMBEDDINGS`, `vt-parser.c:859`). Dispatch is a big `switch` on the trailer in `dispatch_csi()` (`vt-parser.c:1183`).
- **OSC**: `dispatch_osc()` (`vt-parser.c:514`) switches on the numeric code; OSC 8 → `dispatch_hyperlink` (`vt-parser.c:501-510`), OSC 66 → `parse_multicell_code` (`vt-parser.c:628`).
- **DCS**: `dispatch_dcs()` (`vt-parser.c:728`); kitty's own `kitty-…` DCS commands in `parse_kitty_dcs()` (`vt-parser.c:691-725`).
- **APC**: only `G` (graphics) is recognised (`vt-parser.c:1480-1485`). The key=value parsers for APC `G`, OSC 66 and the DnD protocol are **generated C** (`kitty/parse-graphics-command.h:1` "generated by apc_parsers.py", generator `gen/apc_parsers.py`).
- The same translation unit is compiled twice: once normal, once with `DUMP_COMMANDS` to produce a tracing parser (`parse_worker` vs `parse_worker_dump`, `vt-parser.c:1673-1683`, selected at `child-monitor.c:194-195`).

Design decision: optimise the overwhelmingly common case (plain text) with SIMD and batch delivery; keep escape parsing a simple hand-written switch over a contiguous buffer so payloads never need reassembly across reads.

## 3. Terminal state

- `Cursor` (`data-types.h:259-270`): `x, y`, shape, `non_blinking`, and an `sgr` sub-struct (bold, italic, reverse, strikethrough, dim, blink, decoration, `fg`, `bg`, `decoration_fg`). The cursor *is* the current pen. SGR application: `cursor_from_sgr()` (`cursor.c:89`), and `apply_sgr_to_cells()` for DECCARA-style rectangular SGR (`cursor.c:145`).
- Modes: `ScreenModes` is a struct of bools plus `mouse_tracking_mode` / `mouse_tracking_protocol` enums (`screen.h:26-31`). Private modes are encoded as `number << 5` so ANSI and DEC modes share one integer namespace (`modes.h:27-101`). Set/reset in `set_mode_from_const()` (`screen.c:1983-2066`); XTSAVE/XTRESTORE via `saved_modes` (`screen.h:146`, `screen.c:2671-2724`).
- Charsets: `CharsetState { zero, one, current, current_num }` — only G0/G1 (`screen.h:77-79`); designate/shift in `screen.c:1259-1284`; translation applies only to code points < 256 (`map_char`, `screen.c:1288-1290`).
- Tab stops: one allocation of `2 * columns` bools; main and alt halves (`screen.c:178`, `187-190`); default every 8 columns (`screen.c:70-73`); preserved across resize by copying the overlapping prefix (`screen.c:685-703`).
- Saved cursor (DECSC/DECRC): **one `Savepoint` per screen**, not a stack (`main_savepoint`, `alt_savepoint`, `screen.h:138`). It stores cursor, DECOM, DECAWM, DECSCNM and charset (`screen.h:81-86`, `screen.c:2614-2622`, `2706-2721`).
- Per-screen duplicated state swapped on alt-screen toggle: line buffer, tab stops, key-encoding flag stack, graphics manager (`screen.c:1940-1975`).
- Kitty keyboard flag stacks: `main_key_encoding_flags[8]`, `alt_key_encoding_flags[8]` (`screen.h:167`).

## 4. Screen buffer

- **Cell split into two parallel arrays** (`kitty/line.h`):
  - `GPUCell`, 20 bytes: `fg, bg, decoration_fg, sprite_idx, attrs` (`line.h:37-42`). This is exactly what the vertex shader consumes; a line is uploaded with a plain `memcpy` (`screen.c:3903-3906`).
  - `CPUCell`, 12 bytes: 31-bit `ch_or_idx` + 1-bit `ch_is_idx`, 16-bit `hyperlink_id`, `next_char_was_wrapped`, and the multicell fields (`is_multicell, natural_width, scale, subscale_n/d, x, y, width, valign, halign`) (`line.h:50-82`).
  - `CellAttrs` is a 32-bit bitfield: 3-bit decoration (underline style), bold, italic, reverse, strike, dim, blink, 2-bit mark (`line.h:12-25`).
- `LineBuf` (`line-buf.h:13-22`): **one allocation** holding `CPUCell[area]`, `GPUCell[area]`, `line_map[lines]`, `scratch[lines]`, `LineAttrs[lines]` (`line-buf.c:92-103`). Limits: 5000 columns, 50000 lines (`line-buf.c:77`).
- **Scrolling is index rotation, not cell copying**: `linebuf_index()` `memmove`s `line_map` and `line_attrs` only (`line-buf.c:365-374`).
- Soft-wrap marker lives in the **last cell of the line** (`next_char_was_wrapped`, `line-buf.c:207-214`), not in per-line attributes.
- `LineAttrs` (1 byte): `has_dirty_text`, `has_image_placeholders`, 2-bit `prompt_kind` for shell integration (`line.h:84-92`).
- `Line` is a transient *view* (pointers into the buffers), re-pointed with `linebuf_init_line` (`line.h:96-105`, `line-buf.c:147-162`).
- Alt screen: a second `LineBuf` of the same size allocated up front (`screen.c:168-170`); toggle swaps pointers (`screen.c:1940-1975`). Modes 47 / 1047 / 1049 all route there, 1049 additionally saves the cursor and clears (`screen.c:2037-2041`).

Design decision: structure-of-arrays split keeps render data GPU-shaped and contiguous, and keeps text/semantic data off the upload path.

## 5. Scrollback (`kitty/history.c`, `kitty/history.h`)

- `HistoryBuf` is a **ring buffer of fixed-width lines** (`start_of_data`, `count`, `ynum`; `history.h:27-37`). Index 0 is the newest line (`index_of`, `history.c:178-185`).
- Storage is **lazily allocated in segments of 2048 lines** (`SEGMENT_SIZE`, `history.c:17`), each holding CPU cells, GPU cells and line attrs (`history.c:23-25`). Segments are `mmap`ed anonymously "to avoid fragmentation in libc malloc pool" (`history.c:28-30`); added on demand in `segment_for()` (`history.c:52-54`).
- Scrollback uses the **same uncompressed 32 bytes/cell** as the live screen, at full window width. Default `scrollback_lines` is 2000 (`definition.py:523-524`).
- `historybuf_add_line()` copies the line in and prefetches the next slot; the comment claims ~35% bulk-throughput gain (`history.c:331-354`).
- **Pager history**: when the ring evicts a line, it is serialised to ANSI text and appended to a byte ring buffer (`pagerhist_push`, `history.c:298-315`; eviction at `history.c:320-322`). Size is `scrollback_pager_history_size`, default 0 = disabled (`definition.py:745-746`). This second tier is only usable by an external pager, not by in-terminal scrolling; it is re-wrapped as text on width change (`pagerhist_rewrap_to`, `history.c:489`).
- Rendering scrolled-back content reads history lines directly (`render_line_for_virtual_y`, `screen.c:3916-3937`).

## 6. Unicode

- **Property tables are generated**: `gen/wcwidth.py` downloads UCD files from `https://www.unicode.org/Public/…/latest/` (`gen/wcwidth.py:49-50`) — `EastAsianWidth.txt` (`:260`), `GraphemeBreakProperty.txt` (`:280`), `emoji-data.txt` (`:298`), `GraphemeBreakTest.txt` (`:315`) — and emits `kitty/char-props-data.h` ("built from the Unicode Standard 17.0.0", `char-props-data.h:1-2`).
- `CharProps` is a 32-bit packed record per code point: `shifted_width` (3 bits), `is_emoji`, `category`, `is_emoji_presentation_base`, `is_invalid`, `is_non_rendered`, `is_symbol`, `is_combining_char`, `is_word_char`, `is_punctuation`, `grapheme_break`, `indic_conjunct_break`, `is_extended_pictographic` (`char-props.h:13-62`). Lookup is a **3-stage table**, branchless (`char-props.c:11-24`).
- **Width assignment** (`gen/wcwidth.py:1146-1153`): flags / East Asian Wide / wide emoji = 2; marks = 0; non-printing = -1; **East Asian Ambiguous = -2**; private use = -3; unassigned = -4; default 1. `wcwidth_std()` returns `shifted_width - 4` (`char-props.h:166-169`).
  - In the draw loop every negative width becomes 1 (`screen.c:1420-1430`). So **ambiguous-width characters are always narrow**, and there is no ambiguous-width option (no match for "ambig" in `kitty/options/definition.py`).
- **Grapheme segmentation is a table-driven state machine**: `grapheme_segmentation_step(state, props)` is two table lookups returning a 16-bit state with an `add_to_current_cell` bit (`char-props.c:31-38`, `char-props.h:66-103`). It tracks Indic conjunct breaks and emoji modifier/ZWJ sequences in the state bits (`char-props.h:80-85`). The generator tests it against `GraphemeBreakTest.txt` (`gen/wcwidth.py:314-315`, `852`).
- **Draw loop** (`draw_text_loop`, `screen.c:1336-1493`):
  - ASCII fast path: SIMD `printable_ascii_run_length()` then template-`memcpy` of cells for the whole run (`screen.c:1345-1404`).
  - Otherwise: props lookup → segmentation step → if `add_to_current_cell`, append to the previous cell (`screen.c:1410-1419`); zero-width chars that segmentation did not attach are still attached to the previous cell (`screen.c:1421-1428`).
- **Variation selectors**: VS16 after an emoji-presentation base widens the cell to 2 (`screen.c:1215-1218`); VS15 narrows a naturally-wide emoji back to 1 and moves the cursor back (`screen.c:1219-1231`). Width set explicitly via the text sizing protocol wins over VS15 (`screen.c:1221-1224`).
- **Wide characters are "multicell" cells**: both cells carry the same text, `is_multicell=1, width=2`, and the second has `x=1` (`screen.c:1473-1475`). There is no separate "wide spacer" cell kind; the same mechanism serves OSC 66 scaled text.
- **Combining-character storage — TextCache** (`kitty/text-cache.c`): a cell with one code point stores it inline; with more it stores an index (`ch_is_idx`) into a per-screen intern table (array + hash map over an arena, `text-cache.c:29-39`; `add_combining_char`, `screen.c:1094-1103`). Max 24 code points per cell "to prevent DoS attacks" (`line.h:31`, `screen.c:1099`). The cache is **garbage-collected** after 8192 additions by remapping every live cell in history, both line buffers, the paused-render copy and the overlay (`text-cache.c:187-191`, `screen.c:1070-1091`, trigger at `screen.c:1496`).
- A stateful `wcswidth_step()` that mirrors the draw loop's width rules (and skips escape sequences) exists for string measurement (`wcswidth.c:16-110`).
- **Text sizing protocol (OSC 66)** — present (`TEXT_SIZE_CODE 66`, `control-codes.h:237`). Spec: `docs/text-sizing-protocol.rst` (→ `https://sw.kovidgoyal.net/kitty/text-sizing-protocol/`, repo copy checked 2026-10-02). Metadata keys: scale `s` 1–7, width `w`, fractional scale `n`/`d`, vertical/horizontal alignment `v`/`h` (`docs/text-sizing-protocol.rst:66-72`, `147-166`). Text renders in a block of `s*w` by `s` cells (`:93-114`). `w` also lets the application dictate the cell width of a string, which the spec presents as the fix for client/terminal width disagreement (`:12-16`, `:44-46`).
  - Implementation: `screen_handle_multicell_command()` (`screen.c:1619-1662`). With `w=0` the text is segmented into graphemes and each gets its natural width times `s`; with `w>0` the entire payload becomes one multicell. Every covered cell gets a copy of the `CPUCell` with its own `x`,`y` offset (`screen.c:1591-1601`). Bit widths: scale 3 bits, width 3 bits, subscale 4+4 bits (`line.h:44-48`).
  - Cost visible in the source: every editing operation must handle partially-overwritten multicells (the `nuke_multicell_*` family, `screen.c:460-540`), and reflow has a separate slow path (section 7).

## 7. Resize and reflow (`kitty/resize.c`, `screen.c:300-750`)

- `screen_resize()` (`screen.c:630-750`) first cancels synchronized-update pause (`:631`).
- **Main screen + scrollback are re-wrapped together** into freshly allocated buffers: `resize_screen_buffers()` (`resize.c:300-340`) walks history then screen as one logical stream (`resize.c:290-297`), joining lines whose last cell has `next_char_was_wrapped`. Trailing blank lines are excluded (`resize.c:45-48`, `288`) and trailing blanks trimmed per line (`resize.c:85`).
- **Alt screen is not re-wrapped**: `resize_screen_buffer_without_rewrap()` truncates/extends (`screen.c:330`, `resize.c:372-437`).
- Fast paths: identical geometry → `memcpy` (`resize.c:271-281`); lines without multicell content use a bulk range copy (`fast_copy_src_to_dest`, `resize.c:244-265`); lines with multi-row cells take `multiline_copy_src_to_dest` (`resize.c:217`).
- **Cursor tracking**: up to three positions (cursor, main saved cursor, alt saved cursor) are carried through the rewrap as `TrackCursor` entries (`screen.c:310-341`, `resize.c:166`, `323-335`).
- **Prompt protection**: if shell integration marked the current prompt, its lines are saved, blanked before rewrap and copied back unwrapped afterwards, to avoid flicker and double prompts when the shell redraws (`prevent_current_prompt_from_rewrapping`, `screen.c:555-599`; restore at `screen.c:734-748`).
- Images: cell (placeholder) images are dropped and regenerated; the graphics manager is told the before/after content line counts (`screen.c:672-676`).
- Optional: pull lines back from scrollback when the window grows (`scrollback_fill_enlarged_window`, `screen.c:721-731`).
- Tab stops, margins reset and selections cleared (`screen.c:682-705`).

## 8. Input

### Keyboard

- Encoder: `kitty/key_encoding.c`. Entry `encode_glfw_key_event(event, cursor_key_mode, key_encoding_flags, output)` (`key_encoding.c:444-470`) is a **pure function of (key event, DECCKM, flags)**.
- Flags (`key_encoding.c:452-456`), matching the spec (`docs/keyboard-protocol.rst:343-432` → `https://sw.kovidgoyal.net/kitty/keyboard-protocol/`, repo copy checked 2026-10-02):

| Bit | Meaning |
| --- | --- |
| 1 | Disambiguate escape codes |
| 2 | Report event types (press/repeat/release) |
| 4 | Report alternate keys |
| 8 | Report all keys as escape codes |
| 16 | Report associated text |

- Legacy mode is "no flags 1, 2 or 8" (`key_encoding.c:165`); then cursor keys honour DECCKM with `SS3` forms (`key_encoding.c:167-177`) and plain text is sent as text (`SEND_TEXT_TO_CHILD`, `key_encoding.c:460-467`).
- Control sequences (`docs/keyboard-protocol.rst:293-324`): `CSI = flags ; mode u` (set, mode 1 = replace, 2 = OR, 3 = AND-NOT), `CSI ? u` (query), `CSI > flags u` (push), `CSI < n u` (pop). Parser dispatch at `vt-parser.c:1366-1386`.
- State: an 8-entry stack **per screen (main/alt)**, top bit `0x80` marks an occupied slot (`screen.h:167`, `screen.c:2093-2152`). Pushing onto a full stack drops the oldest entry (`screen.c:2137`). Swapped on alt-screen toggle (`screen.c:1952`, `1959`).
- xterm `modifyOtherKeys` is not implemented; enabling it only logs a pointer to the kitty protocol (`screen.c:2082-2091`).

### Mouse

- Tracking modes: `NO_TRACKING, BUTTON_MODE (1000), MOTION_MODE (1002), ANY_MODE (1003)`; protocols: `NORMAL, UTF8 (1005), SGR (1006), URXVT (1015), SGR_PIXEL (1016)` (`data-types.h:103-104`, `modes.h:62-69`, mapping at `screen.c:2000-2006`).
- **X10 mode 9 (press-only) is not defined**: `modes.h` has no mode 9 and `screen.c` has no handler for it.
- Encoder `encode_mouse_event_impl()` (`mouse.c:73-121`): SGR `CSI < cb ; x ; y M/m`; SGR-pixel reuses the SGR format with pixel coordinates (`mouse.c:95-99`); legacy encoding refuses coordinates > 223 (`mouse.c:110`); release is button 3 for pre-SGR protocols (`mouse.c:89`). A leave event is reported only in SGR-pixel mode (`mouse.c:80-83`).

### Paste

- `Screen.paste()` wraps the data in `CSI 200~` / `CSI 201~` when mode 2004 is set (`screen.c:6830-6832`, constants `modes.h:81-83`).
- **Sanitisation**: in bracketed mode, every occurrence of the end marker (`ESC [ 201~` or `0x9b 201~`) is removed, repeatedly until a fixed point so that nested fragments cannot reassemble (`sanitize_for_bracketed_paste`, `kitty/utils.py:1118-1125`; applied at `kitty/window.py:2301-2302`). Without bracketed paste, newlines are converted to `\r` (`window.py:2303-2306`).
- Policy layer `paste_actions` (default `quote-urls-at-prompt,confirm`, `definition.py:1069-1070`): `confirm` replaces C0 controls and asks the user when the paste contains them; also `replace-dangerous-control-codes`, `replace-newline`, `confirm-if-large`, `filter` (`window.py:2207-2268`).
- Mode 5522 "paste events" is also defined (`modes.h:97-98`).

### Focus

- Mode 1004: `CSI I` / `CSI O` on focus change (`screen.c:6848-6858`).

## 9. Hyperlinks (OSC 8)

- Parse: `parse_osc_8()` extracts `id=` from the colon-separated params and the URL (`vt-parser.c:482-499`).
- **Interning**: the key is the string `"<id>:<url>"` (id capped at 256, key at 2048 bytes; `hyperlink.c:12-13`, `139-143`) in a per-screen pool = array + hash map (`hyperlink.c:15-27`). The cell stores a **16-bit id** (`hyperlink_id_type`, `data-types.h:91`; `CPUCell.hyperlink_id`, `line.h:61`); id 0 = none.
- The active id is screen state (`active_hyperlink_id`, `screen.h:163`) stamped onto every drawn cell (`screen.c:1501`); cleared on alt-screen toggle (`screen.c:1942`).
- **Garbage collection**: every 8192 additions, and when the pool approaches 65535 entries; if still nearly full, links in scrollback are discarded, and as a last resort the new link is dropped (`hyperlink.c:147-158`). GC remaps ids in all live cells (`remap_hyperlink_ids`, `hyperlink.c:100`).

## 10. Images

- Spec: `docs/graphics-protocol.rst` (→ `https://sw.kovidgoyal.net/kitty/graphics-protocol/`, repo copy checked 2026-10-02). Wire format: `APC G <key=value,…> ; <base64 payload> ST`; dispatch at `vt-parser.c:1483`, generated parser `parse-graphics-command.h`, command struct `GraphicsCommand` (`graphics.h:14-45`).
- **Transmission** (`docs/graphics-protocol.rst:330-412`): medium `t` = `d` direct (base64, chunked with `m=1/0`, chunks ≤ 4096 bytes), `f` file, `t` temporary file (deleted after read), `s` POSIX/Windows shared memory. Code: `graphics.c:776-823`; shm path `graphics.c:734-742`. Formats: RGB, RGBA, PNG, optional zlib (`docs:286-320`).
- **Image vs placement**: `Image` owns pixel data/texture and a map of `ImageRef` placements (`graphics.h:109-127`, `68-91`). Actions are routed in `grman_handle_command()` (`graphics.c:2621-2665`): transmit `t`, transmit+display `T`, query `q`, put, delete, animation frames `f` / control `a` / compose.
- One `GraphicsManager` per screen buffer (main and alt, `screen.c:172-173`).
- **Unicode placeholders** (`docs/graphics-protocol.rst:579-600`): the cell contains `U+10EEEE` (`IMAGE_PLACEHOLDER_CHAR`, `data-types.h:163`); diacritics encode row/column, colours encode the image id. The draw loop only sets a per-line flag (`screen.c:1457-1460`); the scan happens **at render time** in `screen_render_line_graphics()` which creates cell-bound refs from a virtual placement (`screen.c:3966-3990`; `is_virtual_ref`, `graphics.h:75-81`). This makes images survive tmux/vim, which move the text around.
- **Limits**: 10000 px per dimension (`graphics.c:27`), 400 MB per transfer (`MAX_DATA_SZ`, `graphics.c:694`), **320 MiB storage quota per buffer** (`DEFAULT_STORAGE_LIMIT`, `graphics.c:28`; documented at `docs/graphics-protocol.rst:1052-1058`). Eviction: unreferenced images first, then transient-then-least-recently-used (`apply_storage_quota`, `graphics.c:319-335`). Animation frames live in an on-disk cache with a 5x quota (`graphics.c:1891-1893`; `disk_cache`, `graphics.h:168`).
- **Sixel**: the string "sixel" does not occur anywhere in the repository at this commit (case-insensitive search over `docs`, `kitty`, `kittens`, `tools` returned nothing), so kitty has no sixel code path and its docs do not discuss it. The maintainer's stated reasons for rejecting sixel are in GitHub issues, which were not consulted: **unverified**.

## 11. Rendering

- **Frame driver**: `render()` (`child-monitor.c:1136-1170`) → `prepare_to_render_os_window()` (`:822`) → `render_prepared_os_window()` (`:1024`). With `sync_to_monitor` (default yes, `definition.py:1495-1496`) and platform support, drawing waits for a compositor/display frame callback (`USE_RENDER_FRAMES`, `child-monitor.c:39`, `1067`, `1093`).
- **Damage tracking is two-level**:
  1. Screen-level flags decide whether to touch the GPU buffer at all: `is_dirty`, `scroll_changed`, `reload_all_gpu_data`, resize (`shaders.c:1090`).
  2. Line-level `has_dirty_text` decides whether a line must be **re-shaped** (`screen.c:4119-4125`).
  When an upload happens, **all visible lines are copied** into a freshly mapped buffer (`alloc_and_map_vao_buffer` … `update_line_data` for every row, `shaders.c:1077-1086`, `screen.c:4100-4128`); there is no partial sub-range upload. Clean lines are a `memcpy` of 20 bytes/cell.
- **Draw model**: instanced rendering, one instance per cell, vertex attributes read straight from the `GPUCell` buffer (`sprite_idx`, three colours) plus a 1-byte-per-cell selection buffer (`configure_cell_vao_attributes`, `shaders.c:1873-1908`). Selection/URL highlighting is a separate buffer so changing it does not dirty text (`shaders.c:1093-1102`, `1125`).
- **Glyph atlas ("sprites")**: a `GL_TEXTURE_2D_ARRAY` of **cell-sized** tiles in sRGB8_A8 (`realloc_sprite_texture`, `shaders.c:264-282`). Allocation is a simple bump counter x → y → z layer (`do_increment`, `fonts.c:324-341`); there is **no eviction**: exhaustion raises "Out of texture space for sprites" (`fonts.c:334-336`). A glyph wider than one cell (ligature, wide char, scaled text) is cut into several cell tiles. The sprite cache key is (glyph ids, ligature index, cell count, scale, subscale, multicell row, alignment) per font (`sprite_position_for`, `fonts.c:348-354`). Decorations (underlines etc.) are sprites too, looked up through a second integer texture (`shaders.c:298-310`).
- **Shaping**: `render_line()` splits a line into runs of the same font (`fonts.c:2188-2232`); each run goes through HarfBuzz (`hb_shape`, `fonts.c:1529`; buffer fill `fonts.c:1212-1225`), then glyphs are grouped back onto cells (`group_normal`, `fonts.c:1765`; Iosevka-style spacer ligatures get their own grouper, `fonts.c:1700`). Results are memoised per font in a **shaped-run cache** keyed by the cell text (`shaped_run_get` / `shaped_run_put`, `fonts.c:1884-1903`; `shaped-run-cache.h`). Ligatures can be disabled under the cursor, which is why a cursor move re-renders the old and new cursor lines (`screen.c:4119`, `shaders.c:1074`, `1090`).
- Box-drawing/Powerline glyphs are drawn procedurally, not from the font (`render_box_cell`, `fonts.c:1132-1180`).
- Shaping and rasterisation happen **on the main thread inside the render path**; only already-rasterised sprites are uploaded (`send_sprite_to_gpu`, `shaders.c:298-324`).
- **Synchronized output (mode 2026)**: `PENDING_MODE 2026` (`control-codes.h:235`) → `screen_pause_rendering()` (`screen.c:2044-2048`, `3781-3840`). On begin, kitty **snapshots** the visible lines, cursor, colour profile, selections and image layers into `paused_rendering` (`screen.h:204-214`, `screen.c:3804-3838`); the parser keeps mutating the live grid, and the renderer draws from the snapshot (`screen.c:4074-4088`, `shaders.c:1071`, `1088-1089`, `1117-1123`). End sets `is_dirty` (`screen.c:3783-3797`). Safety timeout: 2 s default (`screen.c:3802`), checked in `screen_check_pause_rendering` (`screen.c:3760-3762`). Resize cancels it (`screen.c:631`).

---

## What a Rust-core + native-UI terminal should borrow / avoid

### Borrow

| Idea | Why | Source |
| --- | --- | --- |
| I/O thread reads **directly into the parser's buffer**, with back-pressure by not polling when full | No copy, no unbounded queue; a flooding child is throttled by the kernel PTY buffer | `child-monitor.c:1673-1693`, `1832` |
| Small-input fast wake + coalescing window for bulk output | Keeps key-echo latency low without redrawing per `read()` | `child-monitor.c:1907-1919`, `vt-parser.c:1594` |
| "Decode UTF-8 until ESC" as one SIMD primitive, deliver text as a batch; ASCII run fast path in the draw loop | Plain text dominates; makes the state machine trivial | `vt-parser.c:276-291`, `screen.c:1345-1404` |
| Zero-copy, in-place dispatch of OSC/DCS/APC payloads with a hard length cap (and a streaming exception for OSC 52) | No reassembly buffers, bounded memory | `vt-parser.c:451-480` |
| Generated key=value parsers for APC/OSC protocols | The graphics/text-sizing grammars are regular; generation removes hand-written bugs. In Rust this is a macro or build script | `gen/apc_parsers.py`, `parse-graphics-command.h:1` |
| CPU/GPU cell split with a POD render cell | Render data crosses FFI as one flat buffer (ideal for Metal / Direct3D instance buffers); semantic data stays in the core | `line.h:37-82`, `screen.c:3903-3906` |
| Line-map rotation for scrolling | O(lines) index move instead of O(cells) copy | `line-buf.c:365-374` |
| Interned multi-codepoint cell text and interned hyperlinks with small ids in the cell, plus explicit GC | Fixed-size cells; unbounded unique text cannot leak memory | `text-cache.c:29-39`, `screen.c:1070-1091`, `hyperlink.c:136-165` |
| Generated multi-stage Unicode tables and a table-driven grapheme state machine, validated against `GraphemeBreakTest.txt` | One lookup gives width + break class; deterministic and testable. Share the same tables with a `wcswidth` helper so measurement and drawing agree | `char-props.c:20-38`, `gen/wcwidth.py:1122-1153`, `wcswidth.c` |
| One "multicell" representation for wide chars and scaled text | No special wide-spacer cell type; OSC 66 comes almost for free at the storage level | `screen.c:1473-1475`, `1591-1601` |
| Reflow that tracks several cursors, skips the alt screen, and protects the shell prompt | These are the cases that visibly break in naive reflow | `screen.c:310-341`, `555-599`, `resize.c:372` |
| Keyboard encoder as a pure function of (event, DECCKM, flags) with per-screen flag stacks | Trivially unit-testable in the Rust core; the native UI only translates platform key events | `key_encoding.c:444-470`, `screen.c:2093-2152` |
| Bracketed-paste end-marker stripping to a fixed point, plus a separate user-policy layer | The stripping is a security guard; policy (confirm, filter) belongs in the UI | `utils.py:1118-1125`, `window.py:2207-2268` |
| Graphics: image/placement split, per-buffer quota with LRU eviction, dimension and transfer caps, placeholders resolved at render time | Bounded memory; images work through multiplexers | `graphics.c:27-28`, `319-335`, `694`, `screen.c:3966` |
| Mode 2026 as a **snapshot** of the visible state with a timeout | The parser never blocks; the renderer shows a consistent frame; a crashed app cannot freeze the display | `screen.c:3781-3840`, `3760-3762` |
| Shaped-run cache keyed by run text | Shaping is the expensive step when a line is dirty; most runs repeat | `fonts.c:1884-1903` |

### Avoid (or do differently)

| kitty choice | Why not for a Rust core + native UI | Source |
| --- | --- | --- |
| Parsing and state mutation on the UI/main thread | With SwiftUI/WinUI the main thread belongs to the toolkit; heavy output would stall UI events. Parse on a core-owned thread per terminal and hand the UI an immutable snapshot/diff. kitty's own 2026 snapshot code shows the snapshot shape is workable | `child-monitor.c:1576-1577`, `screen.c:3804-3838` |
| Process-global static arrays and a single shared poll thread with a 512-child cap | Global mutable state does not fit Rust ownership or embedding multiple cores; use per-terminal objects | `child-monitor.c:84-88`, `data-types.h:157` |
| Core types are CPython objects (`PyObject_HEAD` in `Screen`, `LineBuf`, `HistoryBuf`, `Cursor`) and callbacks go through Python | Ties the core to an interpreter and its GIL; a Rust core should expose a plain C ABI | `screen.h:122`, `line-buf.h:14`, `history.h:28`, `data-types.h:260` |
| Scrollback stored as full-width uncompressed 32-byte cells; long history only as ANSI text for an external pager | Memory grows as width x lines x 32 B; the second tier is not scrollable in-terminal. Consider compact/paged storage for old lines | `history.c:23-25`, `298-315`, `definition.py:745-746` |
| Full visible-grid re-upload whenever anything is dirty | Fine on OpenGL with a mapped buffer, but with line dirty bits already available a native renderer can upload only dirty rows | `shaders.c:1077-1091`, `screen.c:4100-4128` |
| Cell-sized sprite atlas with no eviction and a hard failure on exhaustion | Atlas is tied to one cell size; scaled text and many fonts multiply entries. Native stacks (Core Text / DirectWrite) favour a glyph atlas with eviction | `shaders.c:264-282`, `fonts.c:324-336` |
| Shaping + rasterisation inside the render path on the main thread | Causes frame hitches on first sight of new glyphs; in a native-UI design this belongs to the renderer side, off the UI thread where possible | `fonts.c:1870-1909`, `shaders.c:298-324` |
| Ambiguous-width always narrow, no setting | CJK users commonly expect a choice; decide the policy explicitly (OSC 66 `w=` is kitty's answer) | `gen/wcwidth.py:1151`, `screen.c:1420-1430` |
| Single DECSC savepoint per screen, G0/G1 only, no X10 mouse mode 9, 7-bit introducers only | Acceptable trade-offs for kitty, but check them against your conformance target (vttest/esctest) before copying | `screen.h:81-86`, `77-79`, `modes.h:61-69`, `vt-parser.c:276-291` |
| Multicell complexity leaking into every edit operation | If OSC 66 is in scope, design the "overwrite part of a multicell" invariant once in the grid API instead of at each call site | `screen.c:460-540`, `resize.c:217`, `255-258` |
| OpenGL-specific draw model | macOS deprecates OpenGL; reuse the *data layout* (instanced cells), not the GL code | `shaders.c:1873-1908` |

Licence note: kitty is GPL-3 (file headers, e.g. `kitty/vt-parser.h:4`). Borrow designs and protocol behaviour from the specs; do not port code unless the new project is GPL-3-compatible.
