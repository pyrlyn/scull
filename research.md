# Terminal emulator architecture — research report

Research for Scull: a terminal with a Rust core and a native UI per platform (SwiftUI on macOS, WinUI on Windows) joined by FFI. Rust computes, the platform renders.

All sources were checked on 2026-10-02. Nothing was built or benchmarked: every performance statement below is a reading of the code, not a measurement, unless it says otherwise.

## 1. Sources

Each terminal was read from a shallow clone at the commit below. The detail files carry `path:line` citations for every claim; this report cites the detail file section instead of repeating them.

| Project | Repository | Commit | Release | Licence | Detail file |
| --- | --- | --- | --- | --- | --- |
| Warp ("Wrapp" in the brief) | https://github.com/warpdotdev/warp | `38b2c55e4ed2` | — | AGPL-3.0-only; `warpui`, `warpui_core` MIT | [warp.md](docs/research/warp.md) |
| kitty | https://github.com/kovidgoyal/kitty | `852fef34534b` | 0.49.2 | GPL-3 | [kitty.md](docs/research/kitty.md) |
| WezTerm | https://github.com/wezterm/wezterm | `cab2516` | no stable release since 2024-02-03 | MIT | [wezterm-alacritty.md](docs/research/wezterm-alacritty.md) |
| Alacritty | https://github.com/alacritty/alacritty | `d692748` | 0.17.0 | Apache-2.0 | same |
| vte | https://github.com/alacritty/vte | `abeae76` | 0.15 | MIT OR Apache-2.0 | same |
| foot | https://codeberg.org/dnkl/foot | `cb27717` | 1.28.0 | MIT | [foot-contour.md](docs/research/foot-contour.md) |
| Contour | https://github.com/contour-terminal/contour | `d8ce17b` | 0.7.0 | Apache-2.0 | same |
| libunicode | https://github.com/contour-terminal/libunicode | `0cf7a3b` | 0.9.3 | Apache-2.0 | same |
| Ghostty | https://github.com/ghostty-org/ghostty | `f523504ea5c9` | — | MIT | [ffi-native-ui.md](docs/research/ffi-native-ui.md) |
| Windows Terminal | https://github.com/microsoft/terminal | `2b5336c1fca1` | — | MIT | same |

The brief names "Wrapp". No terminal of that name exists on GitHub; the creator confirmed it means Warp.

## 2. Headline conclusions

1. **Warp is not a model of core/UI separation.** Its terminal crate imports UI-framework types, its session model lives in the app crate, and its renderer reads the grid under the same mutex the PTY thread writes through. It has no frame snapshot and no render damage tracking (warp.md A.2, A.4). It is a reference for the PTY thread, the event vocabulary, tests and conventions, not for the boundary.
2. **No surveyed terminal does "platform renders".** Ghostty and Windows Terminal both keep the GPU renderer and shaping inside the core; the native layer supplies a view, input, IME and chrome (ffi-native-ui.md §0, Part 1). Scull's requirement is stricter than both.
3. **The closest precedents for our boundary** are Contour's `RenderBuffer` (a resolved snapshot handed to a frontend that only draws) and libghostty-vt's `RenderState` (caller-owned snapshot with per-row dirty flags). Both need the same four changes: flat arrays, per-row generations, a queued event stream instead of callbacks, and platform text stacks instead of the built-in rasteriser (foot-contour.md Part 3a; ffi-native-ui.md Part 1).
4. **No existing Rust core fits.** `alacritty_terminal` is clean and published but stores one `char` per cell, has no images and no input encoders. `wezterm-term` has every feature but is unpublished, heavy, and has no flat buffer. The recommendation is our own state and grid crate on top of leaf crates (wezterm-alacritty.md §4).
5. **Every surveyed terminal has a weakness the others fix**, so "take only the best" is a real design, not a slogan. Section 4 lists the pick per stage.

## 3. Comparison by pipeline stage

| Stage | kitty | WezTerm | Alacritty | foot | Contour | Warp |
| --- | --- | --- | --- | --- | --- | --- |
| Threads | I/O thread pumps bytes; parse, state and render on the main thread | reader and parser threads in `mux`; core is passive | I/O thread parses under `FairMutex` | one thread parses and renders; workers rasterise rows | PTY thread plus render thread, one state mutex | "PTY reader" thread parses under `FairMutex`, 64 KiB per lock hold |
| Parser | SIMD "decode UTF-8 until ESC", in-place OSC/DCS/APC dispatch, 7-bit introducers only | `vtparse` table machine plus typed `Action` enum | `vte` (Williams machine, bulk text path); APC dropped | hand-written state functions, no bulk path | constexpr table plus SIMD bulk text run | `vte` 0.13 fork with APC added |
| Cell | 12-byte CPU cell + 20-byte GPU cell, parallel arrays | 24-byte cell, inline grapheme, boxed fat attributes | 24-byte cell, one `char`, zero-width extras boxed | 12-byte cell, rare data out of line | structure-of-arrays lines with O(1) blank state | alacritty-derived cell |
| Scrollback | ring of mmap'ed 2048-line segments, full cell size | `VecDeque` of lines, clustered form when idle | ring `Storage`, scroll is an offset | one power-of-two ring shared with the screen, lazy rows | line storage with trivial lines | flat text + run-length attributes, 50,000 rows per block |
| Unicode | generated 3-stage tables (Unicode 17), grapheme state machine, ambiguous always narrow | grapheme per cell, Unicode version stack | `unicode-width` per scalar, no clustering | clusters interned, up to 255 code points | libunicode, mode 2027 author, 16 code points | per-scalar width, no clustering, no mode 2027 |
| Reflow | main screen and history together, tracked cursors, prompt protected | per-line rewrap | ring-based reflow | one pass with sorted tracking points; PTY paused while dragging | shrink path does not track the cursor (TODO in source) | index rebuild over flat storage |
| Keyboard | kitty protocol (author), pure encoder | legacy, kitty, win32-input | encoders live in the GUI crate | legacy, kitty | legacy, kitty | legacy, kitty (compiled out on Windows) |
| Hyperlinks | interned ids with GC | attribute on cell | boxed cell extra | per-row sorted ranges | 1024-entry LRU, 16-bit ids | registry |
| Images | kitty graphics; "sixel" absent from the repo | sixel, iTerm2, kitty; sliced per cell | none | sixel as a positioned list | sixel, own protocol; one heap fragment per cell | iTerm2 and kitty behind flags; no sixel |
| Damage | line dirty bits gate reshaping; whole grid re-uploaded | per-line seqno, stable row index | line and column damage bounds | per-row and per-cell dirty bits plus scroll damage | none: snapshot rebuilt in full | none for rendering; 60/s throttle |
| Sync output (2026) | snapshot with 2 s timeout | in `mux`, no timeout | in `vte::ansi`, with timeout | hard timeout | refresh suppressed, 150 ms timeout | buffered in parser driver, 2 MiB / 150 ms caps |

Sources: the numbered sections of each detail file (kitty.md §1–§11; wezterm-alacritty.md §1–§2; foot-contour.md Parts 1–2; warp.md B.1–B.11).

## 4. Best of each — what Scull takes

| Stage | Take from | What exactly | Avoid |
| --- | --- | --- | --- |
| Threading | Warp, Alacritty | Core-owned reader thread per terminal; bounded work per lock hold; `try_lock` with back-pressure; replies written from the same thread | kitty and foot: parsing on the UI thread |
| Parser | kitty, Contour | Bulk text scanner (text until the next control byte, one event per run) in front of a table state machine; in-place payload dispatch with hard caps | foot: per-byte state functions with no bulk path |
| Grid cell | foot, kitty | Small fixed cell; rare data out of line; interned clusters | Contour: owning containers per cell |
| Lines | Contour | O(1) blank and uniform line states | — |
| Scrollback | foot, Alacritty | One power-of-two ring of lazily allocated rows shared by screen and history; scroll is an offset bump | kitty: full-width cells for all history |
| Unicode | kitty, Contour, WezTerm | Generated multi-stage tables validated against `GraphemeBreakTest.txt`; one width authority; mode 2027; Unicode-version and ambiguous-width switch | Warp, Alacritty: per-scalar width |
| Reflow | foot, kitty | Single pass with tracking points (cursor, saved cursor, viewport, selection); alt screen not rewrapped; two-phase interactive resize | Contour: cursor and selection lost on reflow |
| Input | kitty, WezTerm, Contour | Encoder as a pure function of (event, modes, flags); kitty flag stack per screen; win32-input-mode | Alacritty: encoders in the GUI crate |
| Hyperlinks | foot | Per-row sorted ranges; no global table to evict | Contour: bounded LRU referenced from scrollback |
| Images | foot, kitty | Positioned list per grid, image/placement split, quota with LRU eviction | Contour, WezTerm: per-cell fragments |
| Damage | foot, WezTerm | Row dirty bits plus scroll damage; stable row ids and per-row generations | Warp, Contour: none |
| Sync output | kitty, Warp | Snapshot semantics with byte and time caps; no wakeups while buffering | WezTerm: no timeout |
| Boundary | Contour, libghostty-vt | Resolved frame snapshot the UI only draws | Warp: shared mutex read during paint |
| Events | Warp, Ghostty | One payload-free coalesced wakeup plus a polled typed event queue | Contour: callbacks under the state lock |
| Testing | Alacritty, Warp | Recorded stream → expected grid ref tests, in the core crate | Warp: ref tests depend on the whole app crate |

## 5. Warp: core/UI separation and what transfers

How it is split (warp.md A.1–A.4, C.1):

- `warp_terminal` holds PTY, parser driver and grid, but depends on `warpui_core` for colours, pixel units, keystrokes and `AppContext`.
- `TerminalModel` (blocks, alt screen, `ansi::Handler`) lives in the app crate. Both front-ends, GUI and TUI, depend on the whole app.
- Views hold `Arc<FairMutex<TerminalModel>>` and lock it inside `paint`. Warp's own `AGENTS.md` documents double-locking as a deadlock hazard.

Transfers:

1. PTY reader thread design.
2. Payload-free wakeups plus a typed event channel.
3. A narrow input vocabulary (`PtyIntent`) and encoders parameterised by a mode provider.
4. Synchronized output handled in the parser driver with caps.
5. Flat-text scrollback with run-length attributes, as an idea for cold history.
6. Per-entry caps on untrusted input.
7. Ref tests; clippy `-D warnings`; a disallowed-types list; non-fatal invariant reporting.

Does not transfer:

1. The shared mutex: Swift and C# cannot hold a Rust guard. A core-owned snapshot replaces it.
2. No damage tracking: over FFI the marshalling cost dominates, so per-row dirty state is required.
3. UI types in the core.
4. Channels that assume an in-process async executor.
5. Shaping in Rust: with native UI the snapshot carries text runs and cluster boundaries, not glyph ids.
6. Per-scalar width: platform shaping needs grapheme-correct cells.
7. The private shell-integration protocol; OSC 133 and OSC 7 are the public equivalents.

Licence: Warp-original core code is AGPL. Alacritty-derived files must be taken from upstream Alacritty (Apache-2.0), never from Warp's modified copies (warp.md §0, C.4).

## 6. Rendering and tooling

Rendering (ffi-native-ui.md Part 2):

- **macOS:** an `NSView` with a `CAMetalLayer`, a Swift Metal renderer and a CoreText glyph atlas, wrapped in `NSViewRepresentable`. Drawing rows with CoreText is a valid first milestone. Pure SwiftUI is ruled out because IME needs `NSTextInputClient` on an `NSView`; its performance at 200x60 and 120 Hz is **unverified**.
- **Windows:** `SwapChainPanel` with D3D11 and a DirectWrite atlas, as Windows Terminal's AtlasEngine does. TSF and UIA are hand-written. XAML text is not viable for the grid. Authoring WinUI in Rust is not supported: `microsoft/windows-app-rs` is archived.

FFI tooling (ffi-native-ui.md Part 3, crates.io API):

| Tool | Version | Verdict |
| --- | --- | --- |
| cbindgen | 0.29.4 (2026-06-09) | Use: header for Swift and C++ |
| csbindgen | 1.9.8 (2026-05-20) | Use: C# bindings |
| UniFFI | 0.32.2 (2026-09-23) | Not for frames: compound types are serialised into a `RustBuffer`; C# generator is third-party |
| bindsmith (this workspace) | — | Not usable: it generates Dart bindings. Its `generate --check` CI gate is the idea to copy |

Reuse of Rust crates (wezterm-alacritty.md §4):

| Crate | Version | Verdict |
| --- | --- | --- |
| `portable-pty` | 0.9.0 (2025-02-11) | Use as-is; blocking I/O, we add the reader thread. Maintenance to re-check before adopting |
| `vtparse` | 0.7 | Use as-is: has APC and colon sub-parameters; no bulk path, so our scanner goes in front |
| `vte` | 0.15 | Alternative: bulk path built in, but APC is dropped, which blocks kitty graphics |
| `alacritty_terminal` | 0.26.0 | Learn from: ring storage, damage bounds, lease read loop |
| `wezterm-term` and friends | git only | Learn from; fallback is a pinned fork treated as owned code |

### 6.1 Windows host: the C# interop library (T17)

The all-C# host (§8, decision 2) needs Direct3D 11, DXGI and DirectWrite for the renderer, TSF for text input (T18) and UI Automation (T18). Four libraries were compared on 2026-10-08. Versions and dates come from the NuGet registration API (`https://api.nuget.org/v3/registration5-gz-semver2/<id>/index.json`, latest stable entry) and the GitHub REST API (`/repos/<owner>/<repo>`, `/releases/latest`); coverage from each repository's tree on its default branch the same day.

| | Vortice.Windows | TerraFX.Interop.Windows | Microsoft.Windows.CsWin32 | Silk.NET (2.x) |
| --- | --- | --- | --- | --- |
| Latest stable on NuGet | `Vortice.Direct3D11` 3.8.3, 2026-03-04 | 10.0.26100.6, 2025-12-12 | 0.3.358, 2026-10-07 | `Silk.NET.Direct3D11` 2.23.0, 2026-01-23 |
| Repository activity | pushed 2026-10-05; GitHub releases stopped at v1.9.143 (2021), NuGet is the release channel | pushed 2026-07-20; `main` already ported from SDK 10.0.28000.0 | pushed 2026-10-08; release v0.3.355 on 2026-10-07 | pushed 2026-09-30; release v2.23.0 on 2026-01-22; 3.0 developed on `develop/3.0` |
| D3D11, DXGI | yes (`Vortice.Direct3D11`, `Vortice.DXGI`) | yes (`um/d3d11`, `shared/dxgi1_2`, …) | yes (win32metadata partitions `Direct3D11`, `Dxgi`) | yes (`Silk.NET.Direct3D11`, `Silk.NET.DXGI`) |
| DirectWrite | yes, inside `Vortice.Direct2D1` | yes (`um/dwrite`) | yes (partition `DirectWrite`) | yes, inside `Silk.NET.Direct2D` (no `Silk.NET.DirectWrite` package) |
| TSF (`msctf.h`) | no | yes (`um/msctf`, `ITfThreadMgr`, `ITfContextOwner`) | yes (partition `Tsf`: `textstor.h`, `msctf.h`) | no |
| UIA (`UIAutomationCore.h`) | no | no (`ITextProvider`, `IRawElementProviderSimple` absent) | yes (partition `WinAuto`) | no |
| WinUI 3 `ISwapChainPanelNative` | yes (`Vortice.WinUI`) | system XAML only (`windows.ui.xaml.media.dxinterop`) | no | no |
| Shape of a COM object | class over SharpGen's `ComObject`; every returned interface is a managed object | blittable struct with `lpVtbl` and `delegate* unmanaged` calls | `allowMarshaling: false`: blittable structs; default `true`: `[ComImport]` interfaces (built-in COM, RCWs) | blittable struct with `LpVtbl`, `ComPtr<T>` struct handles |
| Allocations per frame | a wrapper per COM object handed out | none from the binding | none with `allowMarshaling: false` | none from the binding |
| AOT and trimming | no `IsAotCompatible` or `IsTrimmable` in its `Directory.Build.props`; depends on `SharpGen.Runtime` 2.4.2-beta, a prerelease | `IsAotCompatible` true; package tags `naot`, `trimmable`; one large assembly, `net10.0` only | source generator and `developmentDependency`: only the APIs named in `NativeMethods.txt` are generated into our assembly, nothing ships beside it | no AOT flag; targets netstandard2.0/2.1, netcoreapp3.1, net5.0 |
| Licence | MIT | MIT | MIT | MIT |

Sources: https://www.nuget.org/packages/Vortice.Direct3D11/3.8.3, https://github.com/amerkoleci/Vortice.Windows (`src/Vortice.Direct3D11/ID3D11Device.cs`, `Directory.Build.props`, `Directory.Packages.props`); https://www.nuget.org/packages/TerraFX.Interop.Windows/10.0.26100.6, https://github.com/terrafx/terrafx.interop.windows (`Directory.Build.props`, `sources/Interop/Windows/DirectX/um/d3d11/ID3D11Device.cs`); https://www.nuget.org/packages/Microsoft.Windows.CsWin32/0.3.358, https://github.com/microsoft/CsWin32 (`src/Microsoft.Windows.CsWin32/settings.schema.json`: `allowMarshaling` "Emit COM interfaces instead of structs", default true), https://github.com/microsoft/win32metadata (`generation/WinSDK/Partitions/{Direct3D11,Dxgi,DirectWrite,Tsf,WinAuto}`); https://www.nuget.org/packages/Silk.NET.Direct3D11/2.23.0, https://github.com/dotnet/Silk.NET (`src/Microsoft/Silk.NET.Direct3D11/Structs/ID3D11Device.gen.cs`).

Two facts narrow what the library has to cover, both from the Windows App SDK reference (checked 2026-10-08):

- A WinUI 3 control exposes UIA text through its XAML automation peer: `Microsoft.UI.Xaml.Automation.Provider.ITextProvider` is a WinRT interface a custom control implements for `AutomationPeer.GetPattern` with `PatternInterface.Text` (https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.automation.provider.itextprovider, Windows App SDK 1.0 to 2.0). It comes with the Windows App SDK projection, not the interop library; raw `UIAutomationCore.h` is a fallback only.
- WinUI 3's `ISwapChainPanelNative` is declared in the Windows App SDK's `microsoft.ui.xaml.media.dxinterop.h`, not the Windows SDK (https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/win32/microsoft.ui.xaml.media.dxinterop/nn-microsoft-ui-xaml-media-dxinterop-iswapchainpanelnative). Neither win32metadata nor TerraFX carries it, so `windows/Scull.App` calls its one method, `SetSwapChain`, through the vtable (T17.3). Its IID is `63aad0b8-7c24-40ff-85a8-640d944cc325`, from `MIDL_INTERFACE` in `include/microsoft.ui.xaml.media.dxinterop.h` of the `Microsoft.WindowsAppSDK.WinUI` 2.3.9 package (https://www.nuget.org/packages/Microsoft.WindowsAppSDK.WinUI/2.3.9, which `Microsoft.WindowsAppSDK` 2.5.1 depends on; checked 2026-10-08); the system XAML interface of the same name has another IID. The panel's COM pointer comes from `WinRT.IWinRTObject.NativeObject.ThisPtr`, present in the `WinRT.Runtime.dll` of `Microsoft.Windows.SDK.NET.Ref` 10.0.19041.57, the projection the .NET 10.0.401 SDK pairs with `net10.0-windows10.0.19041.0` (its `Microsoft.NETCoreSdk.BundledVersions.props`, checked 2026-10-08).
- A composition swap chain takes only `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL` and `DXGI_SCALING_STRETCH`, and software drivers such as `D3D_DRIVER_TYPE_REFERENCE` are not supported for it (https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforcomposition, updated 2022-08-12, checked 2026-10-08). WARP is not named there; `windows/Scull.Render.Tests` creates one on WARP on the Windows CI runner.

**Pick: Microsoft.Windows.CsWin32 with `allowMarshaling: false`.** It is the only candidate that covers all five APIs, TSF and UIA included, from Microsoft's own metadata, and it is the most active (a release the day before this check). With marshalling off every COM interface is a blittable struct called through function pointers, so a frame allocates nothing in the binding, and the generated code is plain unsafe C# that AOT compiles and trimming has nothing to remove from. Being a source generator it adds no runtime assembly, only the APIs listed in `NativeMethods.txt`. TerraFX is the close second (same struct shape, `IsAotCompatible`), but has no UIA and its last release is ten months old. Vortice allocates a wrapper per COM object and leans on a prerelease runtime; Silk.NET 2.x lacks TSF and UIA and is being replaced by 3.0. The risk of CsWin32 is its 0.x version line: generated shapes may change between releases, so the version is pinned exactly and bumped deliberately.

## 7. Architecture plan

```
        Swift (macOS)                         C# / C++ (Windows)
  SwiftUI chrome + NSView surface       WinUI 3 chrome + SwapChainPanel
  Metal + CoreText, NSTextInputClient   D3D11 + DirectWrite, TSF, UIA
              │                                      │
              └────────── C ABI (scull.h) ───────────┘
                               │
   scull-ffi      handles, frame snapshot, event queue, panic guard
   scull-term     session: screens, modes, cursor, selection, search
   scull-input    key, mouse, paste, focus encoders
   scull-grid     cells, rows, ring, reflow, damage, images, links
   scull-unicode  generated width and grapheme tables
   scull-parser   bulk scanner + sequence machine → typed actions
   scull-pty      PTY, reader thread, back-pressure
```

Rules of the boundary (ffi-native-ui.md Part 4):

- **Threads.** The core owns one reader thread per terminal. The UI owns the main and render threads. The core never calls into the UI except one wakeup callback, and never while holding a lock.
- **Frames.** The UI owns a frame object. `scull_frame_update(frame, term)` copies only changed rows into flat buffers under a short lock. Buffers: fixed-size cells, a style table with resolved colours, per-row stable id and generation, text runs for shaping, selection and search highlights, preedit, image placements.
- **Shaping.** The platform shapes. The core alone decides cell widths.
- **Events.** One coalesced wakeup, then a polled queue: title, bell, clipboard read and write (with reply or deny), URL, notification, child exit.
- **Input.** A key struct with physical key, modifiers, consumed modifiers, text, unshifted code point and composing flag. The core encodes bytes.
- **Safety.** `catch_unwind` on every export; a panic poisons one terminal, not the app. Status codes, no unwinding across the boundary.
- **ABI.** Version check at start-up, `struct_size` first in every struct, append-only enums. Generated header and C# bindings are committed and diff-checked in CI.
- **Tests.** Headless: golden frames, esctest and vttest, fuzzing, and a property test that a dirty-row repaint equals a full repaint.

## 8. Decisions

The creator accepted the recommendations below except 2, where the choice is all C#, and 5, where the minimum is macOS 26. The licence (6) is GPL-3.0-or-later.

| # | Decision | Recommendation |
| --- | --- | --- |
| 1 | "Wrapp" means Warp | Confirmed |
| 2 | Windows host language | Recommended: C# WinUI 3 shell plus a C++/WinRT surface control (Windows Terminal's own split). **Creator chose all C#**; the interop library is Microsoft.Windows.CsWin32 (§6.1) |
| 3 | VT core | Our own Rust state and grid on leaf crates, not libghostty-vt (unstable API, adds a Zig toolchain) |
| 4 | PTY | Inside the core |
| 5 | Minimum macOS | Recommended: 14, for `CAMetalDisplayLink`. **Creator chose 26** |
| 6 | Project licence | **Creator chose GPL-3.0-or-later**, with paid subscription services around the terminal. MIT, Apache-2.0 and GPL-3 sources may be ported; Warp's AGPL core stays study-only |
| 7 | Split panes on Windows | One swap chain per window, panes as viewports; Microsoft advises at most four swap chains |

## 9. Unverified

- Performance of every design named above; no benchmark was run.
- kitty's position on sixel: the word does not appear in the repository.
- How much of esctest and vttest runs headlessly.
- The six items listed at the end of foot-contour.md, and the "Not verified" sections of warp.md and wezterm-alacritty.md.
- The C# interop library for the Windows host was compared on paper (§6.1); its per-frame cost on a real renderer is not measured yet (T17.4).
- Whether `d3dcompiler_47.dll`, which `windows/Scull.Render` loads for `D3DCompile`, is guaranteed on every Windows 10 and 11 install. The API reference names the DLL (https://learn.microsoft.com/en-us/windows/win32/api/d3dcompiler/nf-d3dcompiler-d3dcompile, updated 2024-02-22) and "Where is the DirectX SDK?" calls the Windows SDK's copy a redistributable, not a system component (https://learn.microsoft.com/en-us/windows/win32/directx-sdk--august-2009-, updated 2025-03-11; both checked 2026-10-08); that a copy sits in System32 is only observed on the CI runner (T17.2).
