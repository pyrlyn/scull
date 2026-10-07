//! Reference-terminal lookup. `scull-pty` owns session children and has no
//! timed feed of another emulator, so this bench spawns the comparison
//! programs itself.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::Row;

/// How long a headless feed may run before it is `unavailable`.
const FEED_TIMEOUT: Duration = Duration::from_secs(5);
/// Sleep between polls so the wait loop is not a busy spin.
const POLL: Duration = Duration::from_millis(10);
/// kitty documents `--start-as=hidden` as a windowless start. The other
/// reference terminals have no stdin feed that stays off-screen.
const KITTY_HEADLESS: &[&str] = &["--start-as=hidden"];
/// Warp's macOS binary is not installed on `PATH`.
const WARP_APP: &str = "/Applications/Warp.app/Contents/MacOS/stable";

struct Spec {
    name: &'static str,
    program: &'static str,
    fallback: Option<&'static str>,
    headless: Option<&'static [&'static str]>,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "kitty",
        program: "kitty",
        fallback: None,
        headless: Some(KITTY_HEADLESS),
    },
    Spec {
        name: "WezTerm",
        program: "wezterm",
        fallback: None,
        headless: None,
    },
    Spec {
        name: "Alacritty",
        program: "alacritty",
        fallback: None,
        headless: None,
    },
    Spec {
        name: "foot",
        program: "foot",
        fallback: None,
        headless: None,
    },
    Spec {
        name: "Contour",
        program: "contour",
        fallback: None,
        headless: None,
    },
    Spec {
        name: "Warp",
        program: "warp",
        fallback: Some(WARP_APP),
        headless: None,
    },
];

pub(crate) const COUNT: usize = SPECS.len();

pub(crate) fn rows(input: &[u8]) -> Vec<Row> {
    SPECS.iter().map(|spec| row_for(spec, input)).collect()
}

pub(crate) fn machine() -> String {
    format!("{} / {}", uname("-m"), uname("-s"))
}

fn row_for(spec: &Spec, input: &[u8]) -> Row {
    let Some(program) = locate(spec) else {
        return Row::not_installed(spec.name, input.len());
    };
    let Some(args) = spec.headless else {
        return Row::unavailable(spec.name, input.len(), "no headless stdin feed");
    };
    match feed_stdin(&program, args, input) {
        Ok(elapsed) => Row::measured(spec.name, input.len(), elapsed),
        Err(reason) => Row::unavailable(spec.name, input.len(), reason),
    }
}

fn locate(spec: &Spec) -> Option<PathBuf> {
    if let Some(path) = on_path(spec.program) {
        return Some(path);
    }
    // The terminal build is sometimes named apart from the cloud CLI.
    if spec.program == "warp"
        && let Some(path) = on_path("warp-terminal")
    {
        return Some(path);
    }
    spec.fallback.and_then(|fallback| {
        let path = PathBuf::from(fallback);
        path.is_file().then_some(path)
    })
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn uname(flag: &str) -> String {
    Command::new("uname")
        .arg(flag)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn feed_stdin(program: &Path, args: &[&str], bytes: &[u8]) -> Result<Duration, &'static str> {
    let started = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "failed to start")?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("no stdin");
    };
    let payload = bytes.to_vec();
    let writer = thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&payload);
    });
    let outcome = wait_timeout(&mut child);
    let _ = writer.join();
    outcome?;
    Ok(started.elapsed())
}

fn wait_timeout(child: &mut std::process::Child) -> Result<(), &'static str> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("exited nonzero"),
            Ok(None) if started.elapsed() >= FEED_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out");
            }
            Ok(None) => thread::sleep(POLL),
            Err(_) => return Err("wait failed"),
        }
    }
}
