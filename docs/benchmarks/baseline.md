# Baseline

One wall-clock feed of the same 1000000 bytes on 2026-10-07, arm64 / Darwin. The Scull row is `Stub::new(80, 24).feed` from `scull-harness`. A missing binary is `not installed`. A terminal with no headless stdin feed is `unavailable` and is not compared. Numbers are this run only.

Reproduce with `just bench` (`cargo run -p scull-bench --release --locked`).

| terminal | status | bytes | seconds | MiB/s |
| --- | --- | --- | --- | --- |
| scull stub | ok | 1000000 | 0.001422 | 670.56 |
| kitty | not installed | 1000000 | - | - |
| WezTerm | not installed | 1000000 | - | - |
| Alacritty | not installed | 1000000 | - | - |
| foot | not installed | 1000000 | - | - |
| Contour | not installed | 1000000 | - | - |
| Warp | unavailable: no headless stdin feed | 1000000 | - | - |
