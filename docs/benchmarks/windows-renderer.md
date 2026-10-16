# Windows renderer

Measurements of the Direct3D 11 renderer (`windows/Scull.Render`) on the T3 input, taken by `windows/Scull.Bench` on 2026-10-16 in [CI run 38888536284](https://github.com/pyrlyn/scull/actions/runs/38888536284), job `rust (windows-latest)`, step `just windows-bench`. **These are numbers from a shared GitHub Actions VM** (Windows 10.0.26100, AMD EPYC family 25, 4 logical processors), single runs, with no physical GPU known to be present. Read them as orders of magnitude, not as a regression gate, and not as what a desktop with a GPU does.

## Method

The same as the macOS table in [macos-renderer.md](macos-renderer.md), from `RendererBenchmark.swift`: the T3 input of [baseline.md](baseline.md) (1000000 bytes, A to Z, a line feed closing every 80 bytes) fed through the real core (release `scull_ffi.dll`: parser, grid, frame updates) into an 80 x 24 terminal in chunks. With rendering, every chunk's frame is drawn offscreen by `Scull.Render` at Consolas 28 px (14 points at a scale of 2, as on macOS), atlas and shaper warm.

What differs from macOS: Direct3D queues its work, so the CPU does not wait for the GPU on its own. Rows marked "queued" read the target back once at the end, which waits for all frames, so their ms per frame is throughput. Rows marked "waited each frame" read it back after every frame, which is the time of one frame from start to finish including the copy of the target to the CPU. macOS waited every frame in all render rows.

Reproduce with `just windows-bench` (Windows only; the table lands in `target/windows-renderer-bench.md`). The input generator is `scull_bench::fixed_input`; `windows/Scull.Bench` holds a C# copy, and both test suites pin the same hash of it.

## WARP (software rasteriser)

| case | chunk | frames | seconds | MiB/s | ms per frame |
| --- | --- | --- | --- | --- | --- |
| feed, update | 4096 | 0 | 0.062310 | 15.31 | - |
| feed, update, render (queued) | 4096 | 245 | 0.948454 | 1.01 | 3.871 |
| feed, update | 65536 | 0 | 0.043948 | 21.70 | - |
| feed, update, render (queued) | 65536 | 16 | 0.095171 | 10.02 | 5.948 |
| one row changed (queued) | - | 200 | 0.584912 | - | 2.925 |
| one row changed (waited each frame) | - | 200 | 1.366594 | - | 6.833 |
| all rows changed (queued) | - | 200 | 1.176397 | - | 5.882 |
| all rows changed (waited each frame) | - | 200 | 0.971890 | - | 4.859 |

## Hardware adapter

The bench asks for a hardware device after WARP and got one. The runner is a VM without a known GPU, so this may be a software or virtual adapter; the bench does not record the adapter's name yet, and the numbers sit close to WARP's.

| case | chunk | frames | seconds | MiB/s | ms per frame |
| --- | --- | --- | --- | --- | --- |
| feed, update | 4096 | 0 | 0.038596 | 24.71 | - |
| feed, update, render (queued) | 4096 | 245 | 0.878811 | 1.09 | 3.587 |
| feed, update | 65536 | 0 | 0.067794 | 14.07 | - |
| feed, update, render (queued) | 65536 | 16 | 0.077940 | 12.24 | 4.871 |
| one row changed (queued) | - | 200 | 0.439515 | - | 2.198 |
| one row changed (waited each frame) | - | 200 | 0.636579 | - | 3.183 |
| all rows changed (queued) | - | 200 | 0.521036 | - | 2.605 |
| all rows changed (waited each frame) | - | 200 | 0.670636 | - | 3.353 |

## Reading it

- The feed alone (core, parser, frame update) runs at 14 to 25 MiB/s here against 21 to 26 MiB/s on the M3 Max, so the core is in the same range on this VM.
- A frame costs 2 to 6 ms on this VM against 0.5 to 0.7 ms on the Apple GPU. At 60 Hz (16.7 ms) that still leaves room, but the renderer, not the core, is what limits throughput of the 4096-byte case here (1 MiB/s), the reverse of macOS.
- The waited and queued rows disagree in both directions (one row: waited is slower, all rows on WARP: waited is faster), which says these single runs on a shared VM carry noise of the same size as the differences.

## Left to measure

- The same on a desktop with a physical GPU, and the adapter's name recorded in the table.
- Key to frame latency on Windows (the macOS probe has no twin yet).
- The T3 reference terminals, as on macOS (T3).
