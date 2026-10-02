# Rust core + native UI (SwiftUI / WinUI) over FFI — research findings

All sources checked on **2026-10-02**. Source-level citations use these shallow clones:

- Ghostty: `ghostty-org/ghostty` @ `f523504ea5c9f41d150d1eb93cc7a748b90f9361` (commit date 2026-10-02) — cited as `ghostty:<path>:<lines>`
- Windows Terminal: `microsoft/terminal` @ `2b5336c1fca1e53ceeaac09710a13c478938cc8d` (commit date 2026-09-30) — cited as `wt:<path>:<lines>`

Clones live in `scratchpad/src/{ghostty,terminal}`. Anything marked **unverified** has no primary source behind it. Sections titled "Recommendation" / "Proposed" are design judgement, not facts.

---

## 0. Headline conclusions

1. **Neither Ghostty nor Windows Terminal does "platform renders".** In both, the portable core owns the GPU renderer and the font shaper; the native UI layer only supplies a view/swap-chain host, input, IME, accessibility and window chrome. The product requirement here (Rust computes, platform renders) is a *different* split, and its closest real precedent is **libghostty-vt's `RenderState` API** — a pull-based snapshot with two-level dirty tracking designed for third-party renderers.
2. **The frame path must be a hand-designed C ABI with flat `#[repr(C)]` buffers.** Every binding generator that produces idiomatic Swift/C# either serializes compound values (UniFFI) or lacks one of the two target languages. `cbindgen` (C header → Swift via module map, C++ directly) plus `csbindgen` (C#) on top of one hand-written `extern "C"` surface is the only combination that covers Swift + C# + C++ with zero-copy.
3. **Pull, not push.** One coalesced `wakeup` callback from the core's IO thread; everything else (frames, events) is pulled by the UI on its own thread. This is what Ghostty does for actions (`wakeup_cb` → `ghostty_app_tick`) and what libghostty-vt does for frames.
4. **Both platforms need a classic native view under the declarative UI.** macOS: `NSView` (for `NSTextInputClient`, a Metal layer, `NSAccessibility`) wrapped in `NSViewRepresentable`. Windows: `SwapChainPanel` plus direct TSF and UIA providers. Pure SwiftUI `Text`/`Canvas` or XAML text cannot host IME for a custom text surface.
5. **`bindsmith` is not usable here** (it generates Dart bindings for Flutter); only its CI ideas transfer.

---

## Part 1 — Prior art

### 1.1 Ghostty (Zig core, Swift/AppKit+SwiftUI macOS app)

**Status of the C API.** `include/ghostty.h` calls itself "libghostty-internal": the only consumer is the macOS app, it is "not designed for external use", most functions are undocumented, and external embedders are pointed at `libghostty-vt` instead (`ghostty:include/ghostty.h:1-11`). The header is 1286 lines.

**Shape of the API** (`ghostty:include/ghostty.h`):

| Concern | API | Lines |
| --- | --- | --- |
| Opaque objects | `ghostty_config_t`, `ghostty_app_t`, `ghostty_surface_t`, `ghostty_inspector_t`; `_new`/`_free` pairs | 1145-1146, 1162-1164, 1178-1180 |
| Runtime callbacks (core → host) | `ghostty_runtime_config_s { userdata, supports_selection_clipboard, wakeup_cb, action_cb, read_clipboard_cb, confirm_read_clipboard_cb, write_clipboard_cb, close_surface_cb }` | 1070-1102 |
| Event-loop pump | `ghostty_app_tick(app)` | 1165 |
| Surface creation | `ghostty_surface_config_s { platform_tag, platform (union: `nsview` / `uiview`), userdata, scale_factor, font_size, working_directory, command, env_vars, initial_input, ... }` | 508-521 |
| Geometry | `ghostty_surface_set_size(px,px)`, `_set_content_scale`, `_set_focus`, `_set_occlusion`, `_set_display_id` | 1189-1192, 1244 |
| Keyboard | `ghostty_surface_key(surface, ghostty_input_key_s)`; struct = `{ action, mods, consumed_mods, keycode, text, unshifted_codepoint, composing }` | 391-399, 1200 |
| Committed text / IME | `ghostty_surface_text(ptr,len)`, `ghostty_surface_preedit(ptr,len)`, `ghostty_surface_ime_point(&x,&y,&w,&h)` | 1204-1205, 1220 |
| Mouse | `ghostty_surface_mouse_button/_pos/_scroll/_pressure`, `_mouse_captured` | 1206-1219 |
| Selection / text readback | `ghostty_surface_has_selection`, `_read_selection`, `_read_text`, `_free_text` | 1236-1241 |
| Clipboard (async reply) | `ghostty_surface_complete_clipboard_request`, `_deny_clipboard_request` | 1230-1234 |
| Drawing | `ghostty_surface_draw`, `ghostty_surface_refresh` | 1187-1188 |
| Actions out | `ghostty_action_s { tag, union }` delivered through `action_cb(app, target, action)`; ~70 tags: `NEW_WINDOW`, `NEW_TAB`, `NEW_SPLIT`, `SET_TITLE`, `PWD`, `MOUSE_SHAPE`, `RING_BELL`, `OPEN_URL`, `DESKTOP_NOTIFICATION`, `COLOR_CHANGE`, `SCROLLBAR`, `START_SEARCH`, `SEARCH_TOTAL`, `PROGRESS_REPORT`, `RENDER`, ... | 949-1018, 1089-1091 |

Observations that matter for our design:

- **One tagged-union "action" channel** carries everything the core wants the host to do — window management, title, bell, URL open, notifications, search results. The host switches on the tag.
- **Key events carry both layers**: a physical `keycode`, `mods` and `consumed_mods`, the produced `text`, the `unshifted_codepoint`, and a `composing` flag. The core does the encoding (legacy / Kitty protocol).
- **Clipboard is a callback + completion pair** because reading may need user confirmation (OSC 52).

**Who owns the renderer: the core.** The host passes a raw `NSView*` (`ghostty_platform_macos_s.nsview`, `ghostty:include/ghostty.h:489-491`). The Zig Metal backend takes that view, creates its own `IOSurfaceLayer`, and makes the view layer-hosting by assigning `layer` then `wantsLayer = true` (`ghostty:src/renderer/Metal.zig:79-131`). Font shaping is also in the core: `src/font/shaper/{coretext,harfbuzz,noop,web_canvas}.zig` exist in the tree. So on macOS the Zig core calls CoreText and Metal itself; Swift never sees a cell.

**Threading.** Each surface owns two core threads: `renderer_thr` and `io_thr` (`ghostty:src/Surface.zig:99,135,726-737`). The renderer thread is driven by a coalescing `wakeup` async handle and a `draw_now` handle fed by a `CVDisplayLink` (`ghostty:src/renderer/Thread.zig:39-59`; `ghostty:src/renderer/generic.zig:41-42,221-224,1050-1055`). Each frame, the renderer copies what it needs out of terminal state inside a short critical section ("Update all our data as tightly as possible within the mutex", `ghostty:src/renderer/generic.zig:1366-1388`, into a `terminal.RenderState`). The app runtime calls the host's `wakeup` from arbitrary threads (`ghostty:src/apprt/embedded.zig:48-50,269-270`), and the Swift side answers by hopping to main and calling `ghostty_app_tick` (`ghostty:macos/Sources/Ghostty/Ghostty.App.swift:61-62,119-121,539-546`). Action callbacks then run on the main thread inside the tick; Swift handlers that touch UI still re-dispatch with `DispatchQueue.main.async` in many places (same file, e.g. 1570, 2113, 2181).

**What Swift does.** `SurfaceView` is an `NSView` subclass wrapped for SwiftUI via `NSViewRepresentable` (`ghostty:macos/Sources/Ghostty/Surface View/SurfaceView.swift:544-558`). It implements:

- keyboard: `keyDown` → `interpretKeyEvents` → accumulates IME output → `ghostty_surface_key` (`.../SurfaceView_AppKit.swift:1101-1103,1156-1179,1230,1490-1528`);
- IME: `NSTextInputClient` conformance (`:1918-1920`), `setMarkedText` → `ghostty_surface_preedit` (`:1942,2139-2145`), `firstRect(forCharacterRange:)` → `ghostty_surface_ime_point` (`:2012-2042`), `insertText` → `ghostty_surface_text` (`:2070,2236`);
- accessibility: `accessibilityRole`, `accessibilityValue`, `accessibilitySelectedText`, `accessibilityString(for:)`, `accessibilityLine(for:)` etc. implemented in Swift over text read back from the core (`:2315-2398`);
- size, display id, occlusion forwarding (`:497,816`; `macos/Sources/Features/Terminal/BaseTerminalController.swift:1289`).

**Boundary summary (Ghostty).** Core: PTY, parser, grid, scrollback, selection, search, key/mouse encoding, config, keybindings, font discovery/shaping/atlas, GPU rendering, render and IO threads. Host: windows/tabs/splits, menus, the `NSView` and its event plumbing, IME protocol, accessibility protocol, clipboard access, notifications, title bar. Rationale stated in the header: the API is "tailored to the needs of the macOS app" (`ghostty:include/ghostty.h:3-6`) — i.e. the split maximizes shared code, including rendering, at the cost of the core depending on platform graphics/text APIs.

### 1.2 libghostty-vt (the standalone VT library)

**Status.** `include/ghostty/vt.h` says: "WARNING: This is an incomplete, work-in-progress API. It is not yet stable and is definitely going to change" (`ghostty:include/ghostty/vt.h:10-11,25-26`). README: `libghostty-vt` "is already available and usable today for Zig and C"; "We haven't tagged libghostty with a version yet" (`ghostty:README.md:157,168`). `lib_version` defaults to `0.0.0` (`ghostty:src/build/Config.zig:41`). Latest Ghostty app tag is `v1.3.1` (`git ls-remote --tags`, 2026-10-02); `build.zig.zon` says `1.3.2-dev`. Headers under `include/ghostty/vt/` total 12,268 lines across ~30 files (`allocator, color, formatter, grid_ref, key, kitty_graphics, modes, mouse, osc, paste, render, screen, search, selection, sgr, snapshot, style, terminal, unicode, wasm, ...`). Example consumers in-tree: `example/c-vt*`, `cpp-vt-stream`, `swift-vt-xcframework`, `wasm-vt`, `zig-vt`.

**API shape** (all `ghostty:include/ghostty/vt/...`):

- `terminal.h`: `ghostty_terminal_new/free/reset/resize/set/get/get_multi`, `ghostty_terminal_vt_write(bytes)`, `ghostty_terminal_scroll_viewport`, grid refs (`:2596-3050`). Host effects are options set with `ghostty_terminal_set`: `WRITE_PTY`, `BELL`, `TITLE_CHANGED`, `PWD_CHANGED`, `CLIPBOARD_WRITE`, `CLIPBOARD_READ`, `DESKTOP_NOTIFICATION`, `PROGRESS_REPORT`, `DEVICE_ATTRIBUTES`, `SIZE`, `COLOR_SCHEME`, scrollback limits, Kitty image limits (`:1633-2093`). There is **no PTY, no thread, no renderer, no font** in this library — the caller feeds bytes and supplies a write-back callback.
- `key/event.h` + `key/encoder.h`: an opaque `GhosttyKeyEvent` with setters for action, key, mods, consumed mods, composing, UTF-8 text, unshifted codepoint (`event.h:315-480`), and `ghostty_key_encoder_encode(encoder, event, out_buf, out_buf_size, &out_len)` writing into a caller buffer; `ghostty_key_encoder_setopt_from_terminal` syncs modes (`encoder.h:132-253`). Mouse has the same shape (`mouse/encoder.h:137-208`).
- `render.h` — **the directly relevant precedent** (`render.h:22-120` doc block, API `:601-1096`):
  - The render state is a caller-owned object updated *from* a terminal: `ghostty_render_state_new`, `_update(state, terminal)`, or two-phase `_begin_update` (needs terminal access) / `_end_update` (uses only state-owned memory) so that "a typical renderer would lock, begin the update, unlock, and then end the update while the IO thread is free" (`:42-52,636-685`).
  - "It only needs read/write access to the terminal instance during the update call" (`:29-34`).
  - **Two-level dirty tracking**: global `DIRTY_FALSE / PARTIAL / FULL` (`:233-243`) plus per-row dirty; the *consumer* clears dirty (`ghostty_render_state_clean`, `:702`); `row_iterator_next_dirty` skips clean rows (`:818-838`).
  - Per-row data: `DIRTY`, `RAW`, `CELLS`, `SELECTION`, `CELLS_RAW`, `VIEWPORT_Y`, `ID` (stable row id) (`:427-474`). Per-cell data: `RAW`, `STYLE`, `GRAPHEMES_LEN`, `GRAPHEMES_BUF`, `BG_COLOR`, `FG_COLOR`, `SELECTED`, `HAS_STYLING`, `GRAPHEMES_UTF8` (`:941-1006`). Batched getters `*_get_multi` exist at every level (`:744,885,1082`).
  - Frame-level data: cols/rows, default fg/bg/cursor colors, palette, cursor style/visible/blinking/password-input/viewport position/wide-tail (`:309-400`).
  - **Synchronized output (mode 2026) is the consumer's problem**: `update` always captures current state; a `RENDER_HOLD` callback tells the renderer when to stop updating (`:76-95`).
  - **Overscan** rows for smooth scrolling (`:97-120`).
  - Kitty graphics are exposed as image handles + placement iterators with pixel/grid rects (`kitty_graphics.h:504-806`), i.e. the core decodes and the renderer uploads.

The iterator/getter style costs one FFI call per cell property unless `CELLS_RAW`/`get_multi` is used; our design should go straight to flat buffers (Part 4).

**Rust bindings exist**: crates `libghostty-vt` 0.2.2 (published 2026-09-28) from `Uzaaft/libghostty-rs` (created 2026-03-21, 391 stars, MIT; crates.io API + GitHub API). It is a third-party repo, not under `ghostty-org`. Its README shows `Terminal::new`, `vt_write`, `on_pty_write`, `RenderState::update`, `RowIterator`, `CellIterator`. This is a build-vs-reuse option for the VT core itself (a Zig build dependency inside a Rust core) — out of scope for this document but worth a decision.

### 1.3 Windows Terminal (C++ core, XAML control)

**It is not WinUI 3.** The control derives from system XAML (`Windows::UI::Xaml::...`, e.g. `wt:src/cascadia/TerminalControl/TermControl.cpp:1249`) with the WinUI **2** controls package `Microsoft.UI.Xaml` 2.8.4 (`wt:dep/nuget/packages.config:10`) and C++/WinRT 3.0.260818.1 (`:7`). Lessons transfer to WinUI 3, but the interop interfaces differ (below).

**Layering** (each layer's header states its role):

- `TerminalCore/Terminal` — buffer + VT state, guarded by a lock (`_terminal->LockForWriting()`, `wt:src/cascadia/TerminalControl/ControlCore.cpp:102-103` and ~20 more call sites).
- `ControlCore` — "encapsulates a `Terminal` instance, a `AtlasEngine` and `Renderer`, and an `ITerminalConnection` ... everything that someone might need to stand up a terminal instance in a control, but without any regard for how the UX works" (`wt:.../ControlCore.h:7-11`). Created at `ControlCore.cpp:102,160,399-400`.
- `ControlInteractivity` — "double-click, right click copy/paste, selection ... a UI framework-independent abstraction. The methods this layer exposes can be called the same from both the WinUI `TermControl` and the WPF control" (`wt:.../ControlInteractivity.h:8-12`).
- `TermControl` — the XAML `UserControl` containing a `SwapChainPanel` (`wt:.../TermControl.xaml:5,1268-1326`).

**Renderer is in the core, XAML only hosts the swap chain.** `AtlasEngine` creates the swap chain itself: for an HWND target `CreateSwapChainForHwnd`, otherwise `DCompositionCreateSurfaceHandle` + `IDXGIFactoryMedia::CreateSwapChainForCompositionSurfaceHandle` with a frame-latency waitable object (`wt:src/renderer/atlas/AtlasEngine.r.cpp:327,365-376,453`). It then calls a "swap chain changed" callback with the `HANDLE` (`AtlasEngine.r.cpp:394-398`; registered in `ControlCore.cpp:451-452`); `TermControl` hops to the UI thread and attaches it with `SwapChainPanel().as<ISwapChainPanelNative2>()->SetSwapChainHandle(handle)` (`TermControl.cpp:1294-1298,1363-1366`). The panel's `CompositionScaleX/Y` and size are pushed back to the core (`TermControl.cpp:1376-1379,2423-2426`). A dedicated render thread paints (`wt:src/renderer/base/renderer.cpp:85,128,158-168`), waiting on `WaitUntilCanRender`.

**Text**: AtlasEngine does its own DirectWrite analysis and shaping (`IDWriteTextAnalyzer1`, `GetGlyphs`, `IDWriteFontFallback::MapCharacters` — with a comment that `MapCharacters` is "awfully slow" — `wt:src/renderer/atlas/AtlasEngine.cpp:49-51,951,998-1016,1082`) and a custom glyph atlas with two backends: `BackendD3D` ("custom, performant text renderer with our own glyph cache") and `BackendD2D` ("pure Direct2D text renderer (for low latency remote desktop and older/no GPUs)") (`wt:src/renderer/atlas/README.md:13-18`). The README also records an architectural regret: the generic `Renderer` breaks the buffer into GDI-style primitives which AtlasEngine rebuilds into DirectWrite runs — "pretty wasteful ... incredibly bug prone. It would be beneficial if the TextBuffer and rendering settings were given directly to AtlasEngine" (`README.md:33`). **Lesson: hand the renderer the grid, not drawing primitives.**

**IME**: TSF is implemented directly, not through XAML text boxes: `src/tsf/Implementation.h` implements `ITfContextOwner`, `ITfContextOwnerCompositionSink`, `ITfTextEditSink` (`wt:src/tsf/Implementation.h:17,38-46`), and the control supplies an `IDataProvider` with `GetHwnd()`, `GetViewport()`, `GetCursorPosition()`, `HandleOutput(text)` (`wt:src/tsf/Handle.h:21-26`; `TermControl.h:24-27,255`; `TermControl.cpp:83-87,159-179`). Four methods are the whole contract: window handle, viewport rect, caret rect, committed text.

**Accessibility**: a XAML `AutomationPeer` wraps a framework-independent UIA `ITextProvider` implementation (`wt:.../TermControlAutomationPeer.h:16-18,73`; `InteractivityAutomationPeer.h:13,55,81`; shared code in `wt:src/types/{TermControlUiaProvider,UiaTextRangeBase,ScreenInfoUiaProviderBase}.*`).

**ConPTY**: `ConptyConnection` creates the pseudoconsole via `ConptyCreatePseudoConsole` with flags (`PSEUDOCONSOLE_INHERIT_CURSOR`, glyph-width mode flags), resizes with `ConptyResizePseudoConsole`, and reads on a dedicated `_OutputThread` using overlapped `ReadFile` (`wt:src/cascadia/TerminalConnection/ConptyConnection.cpp:272-294,412,442,465,737-831`). The repo carries its own `src/winconpty`.

**A C ABI over the same core already exists** — the WPF control. `HwndTerminal.hpp` exports `extern "C"` `__stdcall` functions (`wt:.../TerminalControl/HwndTerminal.hpp:43-57`) and C# P/Invokes them (`wt:src/cascadia/WpfTerminalControl/NativeMethods.cs:174-225`): `CreateTerminal(parent, out hwnd, out terminal)`, `TerminalSendOutput`, `TerminalTriggerResize`, `TerminalDpiChanged`, `TerminalRegisterScrollCallback`, `TerminalRegisterWriteCallback`, `TerminalUserScroll`, `TerminalGetSelection`, `TerminalIsSelectionActive`, `TerminalSendKeyEvent(vkey, scanCode, flags, keyDown)`, `TerminalSendCharEvent`, `TerminalSetTheme`, `TerminalSetFocused`, `DestroyTerminal`. Shape: opaque `IntPtr` handle, two registered callbacks (scroll, write-to-PTY), the rest are calls in. The PTY connection lives on the C# side there (`ITerminalConnection.cs`).

**What it teaches about hosting in WinUI:**

1. The panel is only a compositor slot; rendering runs on its own thread against a composition swap chain and is attached once (and re-attached on device loss) from the UI thread.
2. DPI is the panel's `CompositionScale`, not the window DPI, and changes independently of size (`TermControl.cpp:2399-2426` comment block).
3. IME and UIA must be implemented against Win32/COM interfaces by the control; XAML gives nothing for a custom text surface.
4. Keyboard focus/input on `SwapChainPanel` is awkward — an open TODO debates `KeyDown` vs `PreviewKeyDown` and moving input to the panel (`TermControl.xaml:1242-1244`, TODO GH#4031).
5. WinUI 3 specifics (Microsoft Learn): the native interface is `ISwapChainPanelNative::SetSwapChain` in `microsoft.ui.xaml.media.dxinterop.h`, minimum Windows 10 1809 with Windows App SDK 0.5+ (<https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/win32/microsoft.ui.xaml.media.dxinterop/nn-microsoft-ui-xaml-media-dxinterop-iswapchainpanelnative>). The WinUI 3 `SwapChainPanel` docs say to handle `CompositionScaleChanged`, recommend "no more than four swap chains" updating simultaneously, and offer `CreateCoreIndependentInputSource` to "process input and render to a SwapChainPanel entirely on one or more background threads" (<https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swapchainpanel>, page updated 2026-07-28). Whether WinUI 3 exposes a handle-based `ISwapChainPanelNative2::SetSwapChainHandle` equivalent was **not verified**. The four-swap-chain guidance matters for split panes: many visible panes may need one swap chain with sub-viewports.

### 1.4 Other verified "portable core + native UI" terminals

| Project | What it is (per its own README / GitHub API, 2026-10-02) | Relevance |
| --- | --- | --- |
| `deblasis/wintty` (74 stars, pushed 2026-10-02, latest tag `w1.0.0-rc.7`) | "runs the same `libghostty` core with a WinUI 3 shell and a DirectX 12 (GPU) renderer"; "a thin C# layer over `ghostty.dll` via P/Invoke, same architecture as macOS where Swift wraps the same core"; renderer supports "HWND, SwapChainPanel (composition), and shared texture" surface modes; notes "upstream's libghostty C API is still young and unversioned" | Existence proof for **C# WinUI 3 shell + native core over P/Invoke**. Core renders, not C#. |
| `manaflow-ai/cmux` (27.5k stars, Swift, pushed 2026-10-02) | "Open source Ghostty-based macOS terminal with vertical tabs ..." | Third-party Swift app embedding libghostty. |
| `dededemahendra/term` (0 stars, created 2026-09-18, tags v0.1.0/v0.1.1) | `core/` is a Rust library exposing "a small C ABI (`core/include/termcore.h`)" linked statically; `macos/` is "a thin Swift and AppKit shell that draws the grid with Metal. One instanced draw call per frame, a glyph atlas built with Core Text" | The only verified **Rust core + Swift renders** terminal found. Its header (`termcore.h` @ `ffef837c0b85`) is exactly the pull model: `term_feed(bytes)`, `term_grid(out: *u64, len)` (cells packed into `u64`), `term_dirty_rows(out: *u64, len)` (bitmap), `term_cursor`, `term_modes`, `term_selection_*`, `term_responses(out,len)` (bytes to write back to the PTY), `term_title`, `term_colors`. Brand-new and unproven; use as a sketch, not as evidence of quality. |
| `AwalTerminal/Awal-terminal` (34 stars, last push 2026-04-15, v0.25.2) | "`core/` Rust — terminal emulation, ANSI parsing ...; `app/` Swift — macOS UI, Metal rendering" | Second Rust-core + Swift-Metal example; inactive for ~6 months. |
| `lgztx96/XamlToolkit.WinUI.Terminal` (0 stars, `0.1.0-alpha`) | "Windows Terminal Control (WinUI 3 Port)" | Shows WT's control can be ported to WinUI 3; not evaluated. |
| `migueldeicaza/SwiftTerm` (1.7k stars, v1.19.0 2026-08-18) | Pure-Swift engine with AppKit/UIKit front ends; "Proper CoreText rendering"; "Optional GPU-accelerated rendering via Metal"; README mentions "CoreGraphics and Metal renderers" | Not Rust, but evidence that both CoreText-direct and Metal renderers are shipped in one native terminal. |

No verified **Rust core + WinUI** terminal was found. `Syrtis-Windows` (a non-terminal "WinUI 3 shell on the same Rust parsing core") surfaced in search but was not inspected — **unverified**.

---

## Part 2 — Rendering strategies when the platform renders

Latency/throughput statements below are engineering judgement unless a source is given; no benchmark was run.

### 2.1 macOS

| Option | How | Shaping / ligatures / emoji | IME / a11y | Assessment |
| --- | --- | --- | --- | --- |
| **A. `NSView` + `CAMetalLayer`, Swift Metal renderer, CoreText glyph atlas** | `NSViewRepresentable` wraps an `NSView` whose layer is a `CAMetalLayer` (Apple: "configure an NSView object to use a backing layer and assign a CAMetalLayer"; `MTKView` is the suggested higher-level wrapper — <https://developer.apple.com/documentation/quartzcore/cametallayer>). Frame pacing via `CAMetalDisplayLink` (macOS 14+, gives control over "the timing window and rendering delay", <https://developer.apple.com/documentation/quartzcore/cametaldisplaylink>). | Shape each dirty row's runs with CoreText (`CTLine` "contains an array of glyph runs", <https://developer.apple.com/documentation/coretext/ctline>), rasterize unseen glyphs into an atlas, draw instanced quads. Ligatures and fallback come from CoreText; color emoji need a second (BGRA) atlas. | Same `NSView` adopts `NSTextInputClient` and overrides `NSAccessibility` methods — the Ghostty Swift code is a working template. | **Recommended.** Same architecture as Ghostty's renderer, just written in Swift instead of Zig; also what `dededemahendra/term` does. Highest effort (atlas, shaders, cache invalidation). Caveat from Apple: with default `presentsWithTransaction = false`, Metal content is not guaranteed to land in the same frame as Core Animation content — relevant during live resize (<https://developer.apple.com/documentation/quartzcore/cametallayer/presentswithtransaction>). |
| **B. `NSView.draw(_:)` with CoreText (`CTLineDraw`) / TextKit** | Layer-backed view, draw dirty rows with CoreGraphics. | Best fidelity for free (ligatures, emoji, bidi). | Same `NSView` story as A. | Good **first milestone / fallback**: far less code, correct text. Throughput bounded by CPU rasterization on the main thread; SwiftTerm ships this alongside an optional Metal path. TextKit (`NSTextView`) is the wrong abstraction — a terminal grid is not a flowing document. |
| **C. Pure SwiftUI `Text` / `Canvas`** | `Canvas` is "immediate mode drawing" with a `GraphicsContext` (macOS 12+, <https://developer.apple.com/documentation/swiftui/canvas>). | `Canvas` can draw resolved `Text`; no access to glyph runs. | **Blocker:** Apple's route for a custom text view is to "subclass NSView and implement the NSTextInputClient protocol" (<https://developer.apple.com/documentation/appkit/nstextinputclient>); a SwiftUI `Canvas` cannot be a text input client, so an `NSView` is needed anyway. | **Not viable as the terminal surface.** For 200x60 at 120 Hz (12,000 cells, 8.3 ms budget) no primary benchmark was found; third-party posts reporting SwiftUI `Text` lag with large streaming text and Canvas slowdowns are **unverified** leads only (juniperphoton.substack.com "Text performance issue", Apple forum thread 683293). The IME/accessibility argument alone decides it. Use SwiftUI for everything *around* the surface (tabs, splits, settings, search bar, command palette). |

### 2.2 Windows

| Option | How | Shaping | IME / a11y | Assessment |
| --- | --- | --- | --- | --- |
| **A. `SwapChainPanel` + D3D11 + DirectWrite glyph atlas (AtlasEngine approach)** | Composition swap chain attached through `ISwapChainPanelNative::SetSwapChain`; render thread waits on the frame-latency waitable. | `IDWriteTextAnalyzer` for shaping, `IDWriteFontFallback` for fallback (cache results — WT calls `MapCharacters` "awfully slow"), atlas + instanced quads. | TSF implemented by hand (`ITfContextOwner` etc.), UIA `ITextProvider` behind an `AutomationPeer` — WT is the template. | **Recommended**, proven by WT at scale. Highest effort. Keep a D2D fallback backend in mind (WT keeps one for RDP / no-GPU). |
| **B. Win2D `CanvasControl` / `CanvasSwapChainPanel`** | Win2D is "an easy-to-use Windows Runtime API for immediate mode 2D graphics rendering with GPU acceleration"; NuGet `Microsoft.Graphics.Win2D` 1.4.0 (2026-03-16). | DirectWrite via Win2D text layout objects. | Same hand-written TSF/UIA needed. | Cheap prototype path from C#. Risk: the Win2D WinUI 3 docs still state "Moving Win2D onto WinUI3/Project Reunion is a work in progress. Some features are not supported" (<https://microsoft.github.io/Win2D/WinUI3/html/Introduction.htm>); repo last pushed 2026-03-16. No evidence gathered on per-glyph throughput. |
| **C. XAML text (`TextBlock` per row / `RichTextBlock`)** | Retained-mode elements. | Framework-controlled. | No custom IME hook for a non-`TextBox` surface; Microsoft's custom-input route is `CoreTextEditContext` (`Windows.UI.Text.Core`), documented under `/uwp/api/` (<https://learn.microsoft.com/en-us/windows/apps/develop/input/custom-text-input>, updated 2026-09-28); its usability from an unpackaged WinUI 3 desktop window was **not verified**. WT uses raw TSF with an HWND instead. | **Not viable** for the grid; fine for chrome. |

### 2.3 Host language on Windows, and what it means for calling Rust

Registry facts: `Microsoft.WindowsAppSDK` 2.5.1 stable (2026-09-16); `Microsoft.Windows.CsWinRT` 2.3.1 (2026-07-22; 3.0.0 is preview); `Microsoft.Windows.CppWinRT` 3.0.260818.1 (2026-08-19) — NuGet registration API.

| Host | Calling the Rust C ABI | Rendering code | Notes |
| --- | --- | --- | --- |
| **C# (.NET) + WinUI 3** | `[LibraryImport]` (.NET 7+ source generator: no runtime IL stub, AOT/trim friendly, "allowing the P/Invoke to be inlined"; requires `AllowUnsafeBlocks` — <https://learn.microsoft.com/en-us/dotnet/standard/native-interop/pinvoke-source-generation>). Microsoft guidance: prefer `LibraryImport`, prefer function pointers + `[UnmanagedCallersOnly]` over delegates for callbacks, make structs blittable (avoid `bool`, which marshals as 4-byte `BOOL` by default), use `SafeHandle`, root any delegate handed to native code (<https://learn.microsoft.com/en-us/dotnet/standard/native-interop/best-practices>, updated 2026-07-12). Bindings can be generated by `csbindgen` (emits `DllImport` with `Cdecl`). | D3D11/DirectWrite must be reached through COM interop from C# (a projection or binding library is required; none was evaluated here — **open item**), or via Win2D. | Fastest for app chrome; wintty proves C# WinUI 3 over a native terminal core works, but there the renderer is native. |
| **C++/WinRT + WinUI 3** | Include the cbindgen header and call directly; no marshalling layer. | D3D/DWrite/TSF/UIA are native C++ APIs; WT's AtlasEngine, `src/tsf`, `src/types/*Uia*` (MIT) are directly reusable reference code. | Slower UI iteration; this is WT's own stack. |
| **Hybrid** | A small C++/WinRT *control* component (surface, renderer, TSF, UIA) consumed by a C# WinUI 3 app shell; both call the same Rust DLL. | As C++. | Mirrors WT's `TerminalControl` (WinRT component with `.idl`) vs app split. Two toolchains. |
| **Rust authoring WinUI via windows-rs** | — | — | Not a supported path: `microsoft/windows-app-rs` ("Rust for the Windows App SDK") is **archived**, last push 2022-08-24 (GitHub API). The Learn page for Rust for Windows covers calling Win32/WinRT APIs with the `windows` crate (0.62.2 on crates.io, 2025-10-06; `windows-bindgen` 0.100.0, 2026-09-03; repo release "74", 2026-09-03) and mentions Composition, not XAML/WinUI authoring (<https://learn.microsoft.com/en-us/windows/dev-environment/rust/rust-for-windows>, updated 2026-07-07). |
| **Rust renders on Windows (escape hatch)** | — | Rust uses the `windows` crate for D3D11 + DirectWrite and hands a swap chain to the panel, i.e. WT/wintty's model. | Violates the stated "platform renders" requirement but is the lowest-interop-cost option; record it as a consciously rejected alternative. |

**Recommendation:** decide between "C# shell + C++/WinRT control" and "all C++/WinRT" before designing the Windows renderer; the FFI design in Part 4 is identical for both because it is a plain C ABI. Ask the creator — the requirement says WinUI, not which language.

---

## Part 3 — FFI tooling (versions from crates.io API / GitHub API, 2026-10-02)

| Tool | Latest | Released | Swift | C# | Zero-copy frame path? | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| **cbindgen** (mozilla) | 0.29.4 | 2026-06-09 | via C header + module map (how Ghostty's Swift consumes `ghostty.h`: `include/module.modulemap`) | no (C/C++ only) | Yes — it only describes your own `extern "C"` + `#[repr(C)]` | **Use.** "creates C/C++11 headers for Rust libraries which expose a public C API"; README warns development is "largely adhoc". |
| **csbindgen** (Cysharp) | 1.9.8 | 2026-05-20 (crate) / 05-21 (GitHub, NuGet) | no | yes — "Automatically generates C# `DllImport` code from Rust `extern "C" fn` code", `Cdecl` | Yes (raw pointers/structs) | **Use** for C# host. Emits `DllImport`, not `LibraryImport`; fine for blittable signatures. Repo pushed 2026-09-30. |
| **UniFFI** (mozilla) | 0.32.2 | 2026-09-23 | built-in ("support for Kotlin, Swift, Python and Ruby") | third-party only (`NordSecurity/uniffi-bindgen-cs`) | **No**: "Non-trivial types such as Strings, Optionals and Records, etc. are lowered to a byte buffer called a `RustBuffer`" (<https://mozilla.github.io/uniffi-rs/latest/internals/lifting_and_lowering.html>) — every frame would be serialized and copied | Not for frames. Possible for a control plane, but C# lags: `uniffi-bindgen-cs` latest `v0.11.0+v0.31.0` (2026-06-23) targets uniffi 0.31, one minor behind 0.32.2; not on crates.io (install by git tag); previous release 2025-09-09; .NET 8+; sizes limited to `i32`. UniFFI README: "ready for production use, but ... a long way from a 1.0 release". |
| **swift-bridge** | 0.1.59 | 2026-01-06 | yes (only Swift) | no | README type table lists `&[T]`, `&mut [T]`, `Arc<T>`, `Box<dyn Fn>` as "Not yet implemented" | Covers one platform only and lacks slices; skip. |
| **Diplomat** | 0.16.1 | 2026-08-20 | **no** Swift backend (tool backends: `c, cpp, dart, dotnet, js, kotlin, nanobind`; README lists ".NET (C#)") | yes (`dotnet` backend) | Borrowed slices exist, but "unidirectional" (foreign → Rust only, no callbacks) per its book | C header output could feed Swift, but then it is cbindgen with more constraints; skip. |
| **Interoptopus** | 0.16.5 | 2026-09-15 | no | "Tier 1 ... Full support"; README claims "plain calls are near-zero overhead (1-10ns)"; slices, callbacks, services, async | Yes for C# | Strong **C#-only** option; C and Python backends are "currently suspended". Would force a second definition style for Swift; skip unless C# ergonomics become a pain point. |
| **cxx** (dtolnay) | 1.0.202 | 2026-09-12 | no | no | C++ only | Only relevant if the Windows host is C++/WinRT; a plain C header already works there. Skip. |
| safer-ffi | 0.1.13 stable (0.2.0-rc1, 2026-01-16) | — | C header | C# header generation not verified | — | Not evaluated further. |

**Fit by API class**

- *High-frequency, zero-copy frame snapshot*: only the hand-written `extern "C"` + `#[repr(C)]` route (cbindgen + csbindgen as pure declaration generators). Swift reads imported C structs through `UnsafeBufferPointer`; C# reads blittable structs through pointers / `ReadOnlySpan<T>` with no marshalling (Microsoft: blittable types "do not need to be converted ... and as this improves performance they should be preferred").
- *Low-frequency control API* (create, resize, config, selection, search, paste): the same C ABI is adequate — the surface is ~40 functions (Ghostty exposes 47 `ghostty_surface_*` functions). Adding UniFFI for this half would mean two binding systems, a C# generator one release behind, and two ownership models. **Recommendation: one C ABI for everything; hand-write thin idiomatic wrappers** (`final class Terminal` in Swift, `SafeHandle`-based class in C#), as Ghostty (Swift) and wintty (C#) do.

**bindsmith** (`/Users/listepo/GitHub/listepo/apps/bindsmith`, README + `plan.md` header). It is a Dart CLI that reads one `bindsmith.yaml` and generates **Dart** bindings for native APIs on the six Flutter platforms by driving ffigen, swiftgen, jnigen, winmd and dart-dbus plus a `.d.ts` driver, with a unified IR, review markers and a cross-platform facade; it is work in progress (CLI generates for the C and Objective-C drivers so far). It is **not usable here**: its output language is Dart and its consumers are Flutter apps, whereas this project needs Swift and C#/C++ to call a Rust library — the opposite direction and different languages. What is reusable is the practice, not the code: its `dump`/`diff`/`generate --check` CI gates (fail when the native API moved or checked-in bindings are stale) are exactly the discipline to apply to the generated `term.h` and C# bindings.

---

## Part 4 — Recommended boundary design

Design proposal. Names use a placeholder prefix `tt_`.

### 4.1 Principles (each traced to prior art)

1. **Opaque handles, `_new`/`_free` pairs, userdata pointer** — Ghostty `ghostty_app_t`/`ghostty_surface_t` + `userdata`; WT `HwndTerminal` `void* terminal`.
2. **Core owns IO threads; UI owns render and main threads; they meet at a short lock.** Ghostty's renderer copies state "as tightly as possible within the mutex"; libghostty-vt formalizes it as `begin_update` (locked) / `end_update` (unlocked).
3. **Pull frames, pull events, push only a wakeup.** One C callback, callable from any thread, coalesced; the host hops to its thread and drains. Ghostty: `wakeup_cb` → `ghostty_app_tick`.
4. **The renderer gets the grid, not drawing primitives** — WT's AtlasEngine README regrets the primitive-level interface.
5. **Flat buffers, no per-cell calls** — `dededemahendra/term`'s `term_grid(out,len)`; libghostty-vt's `CELLS_RAW`/`get_multi` escape hatches exist because the iterator API is chatty.
6. **Dirty state is cleared by the consumer** — libghostty-vt `ghostty_render_state_clean`; lets a renderer skip a frame without losing damage.

### 4.2 Objects and threads

```
tt_app      process-wide: config, font-independent settings, wakeup callback, event queue
tt_term     one terminal: PTY + child process, parser, grid, scrollback, selection, search
tt_frame    UI-owned snapshot object (one per renderer), updated from a tt_term
tt_image    refcounted decoded RGBA image (sixel / kitty / iTerm2)
```

| Thread | Owner | Does | May call |
| --- | --- | --- | --- |
| IO thread (one per `tt_term`, or a shared reactor) | core | read PTY, parse, mutate grid under the term lock, write replies, enqueue events, fire `wakeup` | never calls into UI code except `wakeup` |
| UI main thread | platform | input, IME, resize, selection, clipboard, drains events | every `tt_*` function |
| Render thread (optional; display link / waitable swap chain) | platform | `tt_frame_update` + draw | only `tt_frame_*` and `tt_image_*` |

Rules to write into the header:

- All `tt_*` functions are thread-safe with respect to the core's own threads. `tt_term` control calls are serialized by the caller (main thread). `tt_frame` is single-threaded and owned by whoever created it.
- `wakeup(userdata)` may be called from any thread, at any time between `tt_app_new` and `tt_app_free` returning, must not block and must not call back into `tt_*`. It is level-triggered and coalesced (an atomic flag cleared by `tt_app_drain_events` / `tt_frame_update`).
- `tt_term_free` joins the IO thread before returning; no callback fires after `tt_app_free` returns.

### 4.3 Frame snapshot (hot path)

```c
typedef struct tt_frame tt_frame;                 // opaque, UI-owned

tt_frame* tt_frame_new(void);
void      tt_frame_free(tt_frame*);

// Locks the terminal briefly, copies only rows whose generation changed
// into buffers owned by the frame, unlocks. Returns the view below.
// Pointers stay valid until the next tt_frame_update/tt_frame_free on this frame.
tt_status tt_frame_update(tt_frame*, tt_term*, const tt_frame_opts*, tt_frame_view* out);

// Consumer acknowledges it drew everything; clears dirty flags.
void      tt_frame_clean(tt_frame*);

typedef struct {
  uint32_t struct_size;          // ABI versioning: caller sets, core fills min(size)
  uint64_t generation;           // bumps on any visible change; cheap "anything new?" check
  uint32_t cols, rows;           // viewport rows, plus overscan rows if requested
  uint8_t  dirty;                // TT_DIRTY_NONE | PARTIAL | FULL  (libghostty-vt model)
  uint8_t  sync_hold;            // 1 while DEC 2026 synchronized output holds the frame
  uint8_t  _pad[2];
  tt_cursor cursor;              // col,row,style,visible,blinking,wide_tail,color
  tt_colors colors;              // default fg/bg, selection, 256-entry palette ptr
  const tt_row*   rows_ptr;   uint32_t rows_len;
  const tt_cell*  cells_ptr;  uint32_t cells_len;     // rows * cols, row-major
  const tt_style* styles_ptr; uint32_t styles_len;    // style table, id 0 = default
  const tt_run*   runs_ptr;   uint32_t runs_len;      // shaping runs, grouped by row
  const uint16_t* text_ptr;   uint32_t text_len;      // UTF-16 arena for runs/graphemes
  const tt_placement* images_ptr; uint32_t images_len;
  tt_preedit preedit;            // IME marked text to overlay at the cursor
} tt_frame_view;

typedef struct {                 // 24 bytes
  uint64_t id;                   // stable row identity (survives scrolling) -> shaped-row cache key
  uint32_t generation;           // row content version -> cache invalidation
  uint32_t run_start; uint16_t run_count;
  uint16_t flags;                // DIRTY, WRAPPED, HAS_WIDE, HAS_GRAPHEME, HAS_HYPERLINK, PROMPT
  int16_t  sel_start, sel_end;   // selected column range, -1 = none (cf. GhosttyRenderStateRowSelection)
} tt_row;

typedef struct {                 // 8 bytes, blittable in C#, trivially imported by Swift
  uint32_t ch;                   // scalar value, or index into text arena if GRAPHEME flag
  uint16_t style;                // index into styles_ptr
  uint8_t  width;                // 0 = wide-char spacer tail, 1, 2
  uint8_t  flags;                // GRAPHEME, HYPERLINK, SEARCH_MATCH, SEARCH_ACTIVE
} tt_cell;

typedef struct {                 // resolved colors: platform never sees palette indirection
  uint32_t fg, bg, underline_color;   // 0xAARRGGBB, defaults already applied, reverse applied
  uint16_t attrs;                // BOLD, ITALIC, FAINT, BLINK, STRIKE, OVERLINE, INVISIBLE
  uint8_t  underline;            // none, single, double, curly, dotted, dashed
  uint8_t  _pad;
  uint32_t hyperlink_id;         // 0 = none; hover/underline grouping
} tt_style;

typedef struct {                 // one shaping unit: same style, contiguous, same row
  uint32_t text_off; uint16_t text_len;   // UTF-16 slice in text_ptr
  uint16_t col_start, col_count;
  uint16_t style;
  uint8_t  flags;                // ASCII_ONLY (fast path, no shaping), HAS_EMOJI, RTL_HINT
  uint8_t  _pad;
} tt_run;
```

Why this shape:

- **Who shapes: the platform.** The core therefore exposes *text runs + per-cell column widths + styles*, never glyph ids. The core still owns width decisions (wcwidth / grapheme clustering / mode 2027) because they define the grid; the renderer must place each shaped cluster at `col_start + cluster_column` and clip/scale to `width` cells instead of trusting font advances. A UTF-16-index → column map per run is derivable from `cells_ptr` (each cell knows its width and whether it holds a grapheme); if profiling shows the mapping is hot, add an explicit `col_of_utf16` array per run.
- **UTF-16 text arena.** *Assumption to confirm when prototyping:* both shaping stacks take UTF-16 (WT feeds `wchar_t` buffers to `IDWriteTextAnalyzer::GetGlyphs`, `wt:src/renderer/atlas/AtlasEngine.cpp:1082`; CoreText consumes `CFString`/`NSAttributedString`). Emitting UTF-16 once in Rust avoids a per-run transcode in both renderers. Offer `tt_frame_opts.text_encoding` if UTF-8 turns out to be wanted.
- **`ASCII_ONLY` run flag** lets renderers bypass shaping for the common case (direct atlas lookup by code unit) unless ligatures are enabled.
- **`tt_row.id` + `generation`** give the renderer a cache key for shaped rows, so scrolling re-uses shaping work (libghostty-vt exposes a row `ID` for the same reason).
- **Resolved colors** in the style table keep palette/OSC 4/10/11 logic and reverse-video in one place; the palette pointer is still exposed for cursor/selection theming.
- **Selection and search highlights are frame data**, computed in the core (per-row range + per-cell flags), so both renderers agree.
- **Synchronized output**: unlike libghostty-vt (which leaves mode 2026 to the consumer), have `tt_frame_update` return the held frame and set `sync_hold`, with the timeout enforced in the core — one implementation instead of two.
- **Cost model**: one FFI call per frame; `memcpy` of dirty rows only (a full 200x60 repaint is 12,000 x 8 B = 96 KB of cells); zero allocations in steady state because the frame owns reusable buffers.
- **Scrollback / smooth scrolling**: `tt_frame_opts { overscan_above, overscan_below }` as in libghostty-vt; viewport position and total lines are exposed as a `tt_scrollbar { total, offset, len }` (Ghostty sends a `SCROLLBAR` action).

Alternative considered and rejected: **push callbacks with per-row payloads** from the IO thread. It forces the host to copy or lock on a foreign thread, makes backpressure implicit, runs managed code on a Rust thread at high frequency (C# reverse-P/Invoke per row), and loses frame coalescing. Pull + wakeup gives the renderer control of pacing (vsync / 120 Hz) and naturally drops intermediate states under heavy output.

### 4.4 Wakeup and events (cold path, core → UI)

```c
typedef void (*tt_wakeup_fn)(void* userdata);          // any thread, coalesced

typedef struct { uint32_t struct_size; void* userdata; tt_wakeup_fn wakeup; } tt_app_opts;
tt_app* tt_app_new(const tt_app_opts*);

// Main thread: pop events until it returns false. Payload pointers are valid
// until the next tt_app_poll_event call on the same app.
bool tt_app_poll_event(tt_app*, tt_event* out);

typedef struct {
  uint32_t kind;        // tt_event_kind
  tt_term* term;        // NULL for app-level events
  union {
    struct { const uint8_t* utf8; size_t len; }                         title, pwd, notification_body;
    struct { uint8_t target; const uint8_t* data; size_t len; uint8_t mime; } clipboard_write;  // OSC 52 set
    struct { uint64_t request_id; uint8_t target; }                     clipboard_read;   // OSC 52 query / paste
    struct { const uint8_t* uri; size_t len; uint32_t hyperlink_id; }   open_url, hover_link;
    struct { int32_t exit_code; }                                       child_exited;
    struct { uint8_t shape; }                                           mouse_shape;
    struct { uint32_t total; int32_t selected; }                        search_result;
    struct { uint8_t state; uint8_t percent; }                          progress;
    struct { uint16_t index; uint32_t rgb; }                            color_changed;
  } u;
} tt_event;
```

Event kinds (superset drawn from Ghostty's action list and libghostty-vt's terminal options): `FRAME_READY` (implicit in wakeup), `TITLE`, `PWD`, `BELL`, `CLIPBOARD_WRITE`, `CLIPBOARD_READ`, `OPEN_URL`, `HOVER_LINK`, `MOUSE_SHAPE`, `MOUSE_VISIBILITY`, `DESKTOP_NOTIFICATION`, `PROGRESS`, `CHILD_EXITED`, `SELECTION_CHANGED`, `SEARCH_RESULT`, `COLOR_CHANGED`, `SCROLLBAR`, `SECURE_INPUT`, `CELL_SIZE_REQUEST`. `kind` values are append-only.

Why a polled queue instead of Ghostty's `action_cb`: the host gets events on its own thread with no re-dispatch, C# needs exactly one `[UnmanagedCallersOnly]` static function (the wakeup), and nothing managed runs on a Rust thread. Clipboard read is asynchronous by construction: the host answers `CLIPBOARD_READ` with `tt_term_clipboard_reply(term, request_id, data, len)` or `tt_term_clipboard_deny(term, request_id)` — the same request/complete/deny triple as Ghostty (`ghostty.h:1230-1234`), which also gives the host the place to show an OSC 52 confirmation prompt.

### 4.5 Input (UI → core)

```c
typedef struct {
  uint8_t  action;              // PRESS, RELEASE, REPEAT
  uint16_t mods;                // shift/ctrl/alt/super + caps/num lock, with left/right bits
  uint16_t consumed_mods;       // mods the layout already used to produce `text`
  uint32_t key;                 // physical key, layout-independent (W3C UI Events `code`-style enum)
  uint32_t unshifted_codepoint; // logical key without shift, for Kitty protocol alternates
  const uint8_t* text; size_t text_len;   // UTF-8 produced by the layout, may be empty
  bool     composing;           // true while an IME composition is active
} tt_key_event;

bool tt_term_key(tt_term*, const tt_key_event*);        // returns true if consumed/encoded
void tt_term_text(tt_term*, const uint8_t* utf8, size_t len);          // IME commit
void tt_term_paste(tt_term*, const uint8_t* utf8, size_t len);         // core applies bracketed paste + sanitizing
void tt_term_preedit(tt_term*, const uint8_t* utf8, size_t len, uint32_t caret);  // NULL/0 clears
void tt_term_ime_rect(tt_term*, tt_rect_cells* out);                   // cursor cell for candidate window
void tt_term_mouse_button(tt_term*, uint8_t action, uint8_t button, uint16_t mods);
void tt_term_mouse_move(tt_term*, double x_px, double y_px, uint16_t mods);
void tt_term_mouse_scroll(tt_term*, double dx, double dy, uint8_t precise_and_momentum);
void tt_term_focus(tt_term*, bool focused);
```

This is Ghostty's `ghostty_input_key_s` field for field (`action, mods, consumed_mods, keycode, text, unshifted_codepoint, composing`), with one change: the host translates native keycodes to a portable physical-key enum (Ghostty passes the raw platform `keycode`). The core encodes legacy, modifyOtherKeys and Kitty keyboard protocol; the platform never writes escape sequences.

IME: the platform runs the native protocol (`NSTextInputClient` / TSF) and reduces it to four operations, exactly as WT's TSF `IDataProvider` does: *where is the caret* (`tt_term_ime_rect` — the core returns cells, the platform converts to screen points using its own font metrics), *preedit changed*, *commit text*, and *window/view handle* (platform-only). Preedit text is stored in the core only so it comes back inside the frame (`tt_frame_view.preedit`) and both renderers draw it identically; it is never sent to the PTY.

Mouse positions go in as pixels plus the current cell metrics (set via resize), so the core can do SGR-pixel reporting (mode 1016), selection, link hit-testing and drag auto-scroll.

### 4.6 Resize, selection, search, lifecycle

```c
// Platform owns font metrics, so it computes the grid and tells the core both.
void tt_term_resize(tt_term*, uint16_t cols, uint16_t rows,
                    uint32_t width_px, uint32_t height_px, uint16_t cell_w_px, uint16_t cell_h_px);

tt_term* tt_term_spawn(tt_app*, const tt_spawn_opts*);   // command, args, env, cwd, initial size
void     tt_term_free(tt_term*);

void tt_term_scroll(tt_term*, tt_scroll_kind, int32_t amount);   // lines, pages, top, bottom, to-prompt
void tt_term_select(tt_term*, tt_select_kind, ...);              // word, line, all, clear, extend-to
bool tt_term_selection_text(tt_term*, tt_bytes* out);            // core-allocated, tt_bytes_free
void tt_term_search(tt_term*, const uint8_t* utf8, size_t len, uint32_t flags);  // async; results as events
void tt_term_search_next(tt_term*, int32_t direction);
bool tt_term_read_text(tt_term*, tt_text_range, tt_bytes* out);  // accessibility + "copy scrollback"
```

- **Resize** is debounced by the platform only for PTY `SIGWINCH`/`ResizePseudoConsole` purposes if needed; the core reflows and applies the PTY resize. Pixel size is required for `XTWINOPS`/`TIOCGWINSZ` pixel fields and image placement.
- **Selection and search live in the core** (semantic word boundaries, rectangular selection, wrapped-line joining, scrollback search on a core thread). Ghostty and WT both keep them core-side (`ghostty_surface_read_selection`, WT `ControlInteractivity`).
- **Accessibility** is served by `tt_term_read_text` + cursor/selection queries + a `generation` number: the platform implements `NSAccessibility` text methods / UIA `ITextProvider` over those, as Ghostty's Swift and WT's `UiaTextRangeBase` do. Announcing new output for screen readers needs a "text appended since generation N" query — add `tt_term_read_new_output(term, since_gen, out)`.

### 4.7 Images

Core decodes sixel / Kitty graphics / iTerm2 inline images to RGBA8 and owns the bytes. The frame carries placements:

```c
typedef struct {
  tt_image* image; uint64_t image_gen;      // gen changes when pixels change (animation frame)
  int32_t col, row; uint16_t cols, rows;    // destination in cells (may be off-viewport/negative)
  uint32_t src_x, src_y, src_w, src_h;      // source rect in pixels
  int32_t z;                                // below-bg / below-text / above-text
} tt_placement;

void           tt_image_retain(tt_image*);  void tt_image_release(tt_image*);
const uint8_t* tt_image_pixels(tt_image*, uint32_t* w, uint32_t* h, uint32_t* stride);
```

The platform keeps a texture cache keyed by `(image pointer, image_gen)`, retains while cached, and uploads once. This mirrors libghostty-vt's image handle + placement iterator (`kitty_graphics.h:504-806`) with flat data instead of iterators.

### 4.8 Memory ownership rules

1. Whoever allocates frees, through the library's own function. Microsoft's guidance is explicit: "Match the allocator ... Use the library's own free function."
2. **Borrowed-until-next-call**: `tt_frame_view` pointers (until next `tt_frame_update` on that frame) and `tt_event` payloads (until next `tt_app_poll_event`). Zero-copy, no frees.
3. **Core-allocated, caller-freed**: `tt_bytes { const uint8_t* ptr; size_t len; size_t cap; }` returned by `*_text` functions; freed with `tt_bytes_free` (Ghostty: `ghostty_string_free`, `ghostty_surface_free_text`).
4. **Caller-allocated in-parameters** (`text`, `utf8`, env arrays) are copied by the core before the call returns; the core never retains host pointers except `userdata`.
5. **Refcounted**: `tt_image` only.
6. Strings are UTF-8 with explicit length, never NUL-terminated contracts; no `bool` inside structs that C# marshals (use `uint8_t`) — `bool` is "NOT blittable" and defaults to 4-byte `BOOL`. `bool` as a plain return value needs `[return: MarshalAs(UnmanagedType.U1)]` or a `uint8_t` return; prefer the latter.
7. No C `long`, no bitfields, no unions outside `tt_event`; fixed-width integers only (C `long` is 32-bit on Windows, 64-bit on macOS).

### 4.9 Errors and panic safety

- Every fallible function returns `tt_status` (`int32_t`, 0 = OK, as libghostty-vt's `GhosttyResult` and Ghostty's `GHOSTTY_SUCCESS 0`); details via `tt_last_error(tt_bytes* out)` (thread-local) for logging.
- **No unwinding across the boundary.** The Rust Reference states that with `panic=unwind`, a panic reaching an `extern "C"` boundary aborts the process (<https://doc.rust-lang.org/reference/items/functions.html>). An abort kills every tab, so wrap every exported function body in `std::panic::catch_unwind` through one macro, convert a caught panic to `TT_ERR_PANIC`, and **poison that `tt_term`** (subsequent calls return `TT_ERR_POISONED`; the host shows "terminal crashed" in that pane and other panes survive). Do not build the core with `panic=abort`.
- IO-thread panics are caught at the thread root and surfaced as a `CHILD_EXITED`-like `TERM_CRASHED` event.
- Host callbacks (`wakeup`) must not throw/unwind into Rust; in C# use `[UnmanagedCallersOnly]` static methods (Microsoft: prefer function pointers and `UnmanagedCallersOnlyAttribute` over delegates), in Swift a `@convention(c)` closure with no captured context (use `userdata` + `Unmanaged`), as Ghostty's `App.swift:61-62` does.
- Null/invalid handle arguments return `TT_ERR_INVALID` rather than dereferencing; debug builds add a magic-number check in each handle.

### 4.10 ABI versioning

- `uint32_t tt_abi_version(void)` (major << 16 | minor) checked by each host at startup; refuse to run on major mismatch. Wintty's README warning about upstream's "young and unversioned" C API is the cautionary example.
- Every options/output struct starts with `uint32_t struct_size`; the core reads/writes only `min(struct_size, sizeof)` (libghostty-vt does the same with a leading `size` field, e.g. `GhosttyRenderStateRowSelection`, `ghostty:include/ghostty/vt/render.h:499-508`). Fields are append-only; enums are append-only with an explicit `_MAX_VALUE` sentinel to pin enum width (libghostty-vt uses `GHOSTTY_ENUM_MAX_VALUE` on every enum, e.g. `render.h:242`).
- `tt_cell`, `tt_row`, `tt_style`, `tt_run` layouts are part of the major version; add `static_assert(sizeof(tt_cell) == 8)` in the header and mirrored size assertions in Swift (`MemoryLayout`) and C# (`sizeof`) unit tests.
- Generate `term.h` (cbindgen) and the C# bindings (csbindgen) in the build and commit them; CI regenerates and fails on diff (the `generate --check` discipline from bindsmith), plus a symbol-list check of the built `.dylib`/`.dll`.
- Ship as: static lib + header + module map in an XCFramework for Swift (Ghostty has `example/swift-vt-xcframework`), `cdylib` `.dll` for Windows.

### 4.11 Testing the core headlessly

Because the boundary is "bytes in, frame snapshot out", the entire core is testable with no window:

1. **Golden/snapshot tests**: feed a byte fixture to `tt_term`, serialize `tt_frame_view` to a stable text form, compare with `insta` (1.48.0, crates.io updated 2026-06-11). Covers parser, grid, wrap/reflow, styles, selection ranges, dirty flags.
2. **Conformance suites**: `esctest` (<https://gitlab.freedesktop.org/terminal-wg/esctest>, reachable 2026-10-02) drives a terminal over a PTY and checks replies — run it against a headless binary that links the core and answers from the grid; `vttest` (<https://invisible-island.net/vttest/>, reachable 2026-10-02) for scripted screens compared via goldens. How much of either suite passes headlessly is **unverified** until tried.
3. **Fuzzing**: `cargo-fuzz` 0.13.2 (crates.io updated 2026-06-09; <https://rust-fuzz.github.io/book/cargo-fuzz.html>) targets: (a) arbitrary bytes → `feed` then `tt_frame_update` (invariants: no panic, `cells_len == rows*cols`, runs cover each row exactly once, wide cells always followed by a spacer); (b) arbitrary `tt_key_event` → encoder; (c) image decoders.
4. **FFI contract tests**: a small C test program compiled against the generated header exercises create/feed/update/free under ASan/TSan and from two threads; Swift and C# unit tests assert struct sizes/offsets and run one round trip.
5. **Dirty-tracking property test**: after any input, repainting only dirty rows onto the previous frame must equal a full repaint.
6. **Encoder tests**: table-driven key/mouse encoding against xterm and Kitty protocol expectations.
7. Renderer-side golden images (per platform) stay small because shaping/placement inputs are deterministic frames checked in as fixtures.

### 4.12 Open decisions for the creator

1. Windows host language: C# shell + C++/WinRT control, all C++/WinRT, or all C# with a D3D/DirectWrite interop library (none evaluated yet).
2. Build the VT core in Rust from scratch / on existing Rust crates, or embed `libghostty-vt` through the `libghostty-vt` crate (0.2.2; upstream API explicitly unstable, adds a Zig toolchain).
3. PTY inside the core (assumed above; `portable-pty` is 0.9.0, last crates.io update 2025-02-11 — check maintenance before adopting) versus host-supplied byte streams as in libghostty-vt and WT's WPF control.
4. Minimum macOS version: `CAMetalDisplayLink` requires macOS 14.
5. Whether split panes share one swap chain on Windows (the four-swap-chain guidance).
