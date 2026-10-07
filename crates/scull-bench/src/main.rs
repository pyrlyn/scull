//! Writes `docs/benchmarks/baseline.md` and prints the same table.
//! The library does not print.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let path = scull_bench::baseline_path();
    let text = scull_bench::baseline_markdown();
    if let Err(err) = scull_bench::write_baseline(&path, &text) {
        eprintln!("scull-bench: {err}");
        return ExitCode::FAILURE;
    }
    print!("{text}");
    ExitCode::SUCCESS
}
