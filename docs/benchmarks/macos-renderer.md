# macOS renderer

Measurements of the Metal renderer (T15) on 2026-10-08, Apple M3 Max, macOS 27.0.1. Numbers are single runs on a machine with other work going on; read them as orders of magnitude, not as a regression gate.

## Throughput

The T3 input from [baseline.md](baseline.md): the same 1000000 bytes as `scull-bench` (A to Z, a line feed closing every 80 bytes), fed through the real core (`TerminalSession(cols: 80, rows: 24)`, parser, grid and frame updates) in chunks. With rendering, every chunk's frame is drawn offscreen at scale 2 (1440 x 856 pixels) and the CPU waits for the GPU each frame, which is slower than the view's three frames in flight. Atlas and shaper were warm.

Reproduce with `just macos-bench` (Swift release build; the table lands in `target/macos-renderer-bench.md`).

| case | chunk | frames | seconds | MiB/s | ms per frame |
| --- | --- | --- | --- | --- | --- |
| feed, update | 4096 | 0 | 0.045236 | 21.08 | - |
| feed, update, render | 4096 | 245 | 0.262630 | 3.63 | 1.072 |
| feed, update | 65536 | 0 | 0.036745 | 25.95 | - |
| feed, update, render | 65536 | 16 | 0.047090 | 20.25 | 2.943 |
| one row changed | - | 200 | 0.097714 | - | 0.489 |
| all rows changed | - | 200 | 0.134621 | - | 0.673 |

Against T3: the baseline's `scull stub` row (670.56 MiB/s) times `scull-harness`'s stub, not the real core, so it is not comparable with these rows. The reference terminals of T3 (kitty, WezTerm, Alacritty, foot, Contour) were not installed and Warp has no headless feed, so there is no cross-terminal comparison yet.

A frame costs about 0.5 ms when one row changed and 0.7 ms when all did, so at 60 or 120 Hz the renderer is not what limits throughput; the core's feed is.

## Input latency

From the core key event to the frame its echo changes, measured by the debug probe: `Scull -ScullInitialInput 'exec cat' -ScullLatencyProbe <file>` types `x` 100 times, 50 ms apart, and writes the distribution. `cat` leaves the echo to the tty driver; a blank initial input measures the login shell's line editor instead. The app was the `just macos` build (Swift debug, core release).

| path | samples | min | median | p95 | max |
| --- | --- | --- | --- | --- | --- |
| tty echo (`cat`), key to frame rendered, run 1 | 100 | 0.86 ms | 3.40 ms | 22.71 ms | 78.04 ms |
| tty echo (`cat`), key to frame rendered, run 2 | 100 | 0.98 ms | 5.03 ms | 27.44 ms | 42.96 ms |
| zsh line editor (creator's config), key to frame rendered | 100 | 14.26 ms | 30.95 ms | 188.83 ms | 570.71 ms |

"Rendered" is the GPU finishing the frame. The probe also records when the frame reached the screen (`MTLDrawable.presentedTime`), but the window was covered during these runs, so macOS presented nothing and that column has no samples; the probe draws a covered window itself to get the render time.

## Left to measure

- Key to photons on an uncovered window: rerun the probe with the window in front; the on-screen line then fills in. AppKit's event delivery before the core key event is outside the probe either way.
- The same input through the T3 reference terminals once they are installed (T3).
- A release build of the Swift side for the latency runs.
