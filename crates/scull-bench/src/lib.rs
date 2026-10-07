//! Baseline throughput of one fixed byte stream. The timed Scull path is
//! `scull-harness`'s stub; this crate only measures it and writes the table.
//! Printing stays in the binary.

mod terminals;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use scull_harness::Stub;

/// Columns of the stub, and the period of the newline in the fixed input.
const COLUMNS: u16 = 80;
/// Rows of the stub. Bytes past the last row are dropped by the harness.
const ROWS: u16 = 24;
/// Length of the payload every terminal receives.
const INPUT_BYTES: usize = 1_000_000;
/// `A` through `Z`, repeated, so the stream is printable ASCII.
const ALPHABET: usize = 26;
const SECONDS_PER_DAY: u64 = 86_400;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// The fixed payload: `INPUT_BYTES` of a repeating `A`..`Z` pattern, with a
/// newline every [`COLUMNS`] bytes. Built here so the repo does not store it.
#[must_use]
pub fn fixed_input() -> Vec<u8> {
    let mut out = Vec::with_capacity(INPUT_BYTES);
    let mut letter = 0usize;
    while out.len() < INPUT_BYTES {
        if (out.len() + 1).is_multiple_of(usize::from(COLUMNS)) {
            out.push(b'\n');
        } else {
            let offset = letter % ALPHABET;
            out.push(b'A' + u8::try_from(offset).unwrap_or(0));
            letter += 1;
        }
    }
    out
}

/// `Stub::new(80, 24).feed`. The returned count is data-dependent so a
/// release build cannot delete the writes.
#[must_use]
pub fn feed_stub(input: &[u8]) -> usize {
    let mut stub = Stub::new(COLUMNS, ROWS);
    stub.feed(input);
    stub.grid()
        .iter()
        .flatten()
        .fold(0usize, |n, ch| n + usize::from(*ch != ' '))
}

/// Wall-clock duration of one [`feed_stub`] call, for the baseline table.
/// Divan times the same function without this clock.
#[must_use]
pub fn time_stub(input: &[u8]) -> Duration {
    let started = Instant::now();
    let inked = feed_stub(input);
    let elapsed = started.elapsed();
    std::hint::black_box(inked);
    elapsed
}

/// Markdown table for `docs/benchmarks/baseline.md`.
#[must_use]
pub fn baseline_markdown() -> String {
    let input = fixed_input();
    let elapsed = time_stub(&input);
    let mut rows = Vec::with_capacity(1 + terminals::COUNT);
    rows.push(Row::measured("scull stub", input.len(), elapsed));
    rows.extend(terminals::rows(&input));
    render(&rows, &utc_date(), &terminals::machine())
}

/// `docs/benchmarks/baseline.md` relative to this crate.
#[must_use]
pub fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/benchmarks/baseline.md")
}

/// Create parent directories and replace the baseline file.
pub fn write_baseline(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

struct Row {
    terminal: &'static str,
    status: String,
    bytes: usize,
    seconds: Option<Duration>,
}

impl Row {
    fn measured(terminal: &'static str, bytes: usize, seconds: Duration) -> Self {
        Self {
            terminal,
            status: "ok".to_owned(),
            bytes,
            seconds: Some(seconds),
        }
    }

    fn not_installed(terminal: &'static str, bytes: usize) -> Self {
        Self {
            terminal,
            status: "not installed".to_owned(),
            bytes,
            seconds: None,
        }
    }

    fn unavailable(terminal: &'static str, bytes: usize, reason: &str) -> Self {
        Self {
            terminal,
            status: format!("unavailable: {reason}"),
            bytes,
            seconds: None,
        }
    }
}

fn render(rows: &[Row], date: &str, machine: &str) -> String {
    let mut out = String::new();
    out.push_str("# Baseline\n\n");
    out.push_str(&format!(
        "One wall-clock feed of the same {INPUT_BYTES} bytes on {date}, {machine}. \
         The Scull row is `Stub::new(80, 24).feed` from `scull-harness`. \
         A missing binary is `not installed`. A terminal with no headless stdin \
         feed is `unavailable` and is not compared. Numbers are this run only.\n\n"
    ));
    out.push_str(
        "Reproduce with `just bench` (`cargo run -p scull-bench --release --locked`).\n\n",
    );
    out.push_str("| terminal | status | bytes | seconds | MiB/s |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for row in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.terminal,
            row.status,
            row.bytes,
            seconds(row.seconds),
            mib_per_sec(row.bytes, row.seconds),
        ));
    }
    out
}

fn seconds(duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("{:.6}", duration.as_secs_f64()),
        None => "-".to_owned(),
    }
}

fn mib_per_sec(bytes: usize, duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return "-".to_owned();
    };
    let secs = duration.as_secs_f64();
    if secs == 0.0 {
        return "-".to_owned();
    }
    format!("{:.2}", (bytes as f64) / secs / BYTES_PER_MIB)
}

/// UTC calendar date of the run. Howard Hinnant's civil-from-days, so the
/// table does not take a date crate.
fn utc_date() -> String {
    let Ok(since) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return "2026-10-07".to_owned();
    };
    let (year, month, day) = civil_from_days(since.as_secs() / SECONDS_PER_DAY);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: u64) -> (i64, u32, u32) {
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{INPUT_BYTES, Row, fixed_input, render, time_stub};

    #[test]
    fn table_emits_a_missing_terminal_and_a_timed_stub() {
        let input = fixed_input();
        assert_eq!(input.len(), INPUT_BYTES);
        let elapsed = time_stub(&input);
        let rows = [
            Row::measured("scull stub", input.len(), elapsed),
            Row::not_installed("kitty", input.len()),
        ];
        let table = render(&rows, "2026-10-07", "arm64 / Darwin");
        let stub = table
            .lines()
            .find(|line| line.contains("scull stub"))
            .expect("stub row");
        let missing = table
            .lines()
            .find(|line| line.contains("kitty"))
            .expect("missing terminal row");
        assert!(missing.contains("not installed"));
        assert!(missing.contains("| - | - |"));
        assert!(stub.contains(&INPUT_BYTES.to_string()));
        assert!(!stub.contains("| - | - |"));
    }
}
