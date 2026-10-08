//! A child that never stops writing cannot starve a frame read. One thread plays
//! the UI, taking the shared lock the way a frame update does, while a flooding
//! child keeps the reader busy. Separate from `shell.rs` because it is a timing
//! test with its own load and bounds.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use scull_pty::limits::{MAX_LOCKED_BYTES, MAX_PENDING_READ_BYTES};
use scull_pty::{ExitStatus, Mutex, Pty, Sink, SpawnOptions};

/// The shortest flood while frames are read.
const FLOOD_DURATION: Duration = Duration::from_secs(2);
/// The flood goes on past `FLOOD_DURATION` until the run counts: a slow
/// runner reads fewer frames, and `cmd` on ConPTY writes far slower than `yes`.
const MAX_FLOOD_DURATION: Duration = Duration::from_secs(30);
/// How often the test looks whether the run counts yet.
const CHECK_INTERVAL: Duration = Duration::from_millis(100);
/// A UI frame is every ~16 ms; reading faster makes the test harder, not easier.
const FRAME_INTERVAL: Duration = Duration::from_millis(2);
/// The longest a frame read may wait for the lock.
const MAX_FRAME_WAIT: Duration = Duration::from_millis(50);
/// Frames that must have been read for the run to count.
const MIN_FRAMES: usize = 100;

/// Stands in for the terminal: a parse that costs time per byte, so a hold of
/// the full budget is as long as a real one, and counters the frame can read.
#[derive(Default)]
struct Parsing {
    checksum: u64,
    fed: usize,
    since_drain: usize,
    worst_hold: usize,
    /// `fed` for the test thread, which must not take the lock to look.
    flowed: Arc<AtomicUsize>,
    cursor: common::CursorReport,
}

impl Sink for Parsing {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        self.fed += bytes.len();
        self.flowed.store(self.fed, Ordering::Relaxed);
        self.since_drain += bytes.len();
        self.cursor.feed(bytes);
    }

    fn drain_replies(&mut self, out: &mut Vec<u8>, limit: usize) {
        self.cursor.drain(out, limit);
        // Called once at the end of every hold.
        self.worst_hold = self.worst_hold.max(self.since_drain);
        self.since_drain = 0;
    }

    fn child_exited(&mut self, _status: ExitStatus) {}
}

#[cfg(unix)]
fn flooding_child() -> SpawnOptions {
    SpawnOptions::command("yes", &["flood flood flood flood flood flood"])
}

#[cfg(windows)]
fn flooding_child() -> SpawnOptions {
    SpawnOptions::command(
        "cmd",
        &["/C", "for /L %i in (1,0,2) do @echo flood flood flood"],
    )
}

#[test]
fn a_flooding_child_cannot_starve_a_frame_read() {
    let flowed = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(Mutex::new(Parsing {
        flowed: Arc::clone(&flowed),
        ..Parsing::default()
    }));
    let pty = Pty::spawn(&flooding_child(), Arc::clone(&sink), || {}).unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let read = Arc::new(AtomicUsize::new(0));
    let frames = {
        let sink = Arc::clone(&sink);
        let running = Arc::clone(&running);
        let read = Arc::clone(&read);
        thread::spawn(move || {
            let mut worst = Duration::ZERO;
            let mut count = 0;
            while running.load(Ordering::Relaxed) {
                let asked = Instant::now();
                let frame = sink.lock();
                worst = worst.max(asked.elapsed());
                // A frame copies a little out and lets go.
                let _seen = frame.fed;
                drop(frame);
                count += 1;
                read.store(count, Ordering::Relaxed);
                thread::sleep(FRAME_INTERVAL);
            }
            (worst, count)
        })
    };

    let started = Instant::now();
    loop {
        thread::sleep(CHECK_INTERVAL);
        let elapsed = started.elapsed();
        let counts = flowed.load(Ordering::Relaxed) > MAX_PENDING_READ_BYTES
            && read.load(Ordering::Relaxed) >= MIN_FRAMES;
        if elapsed >= MAX_FLOOD_DURATION || (counts && elapsed >= FLOOD_DURATION) {
            break;
        }
    }
    running.store(false, Ordering::Relaxed);
    let (worst, count) = frames.join().unwrap();
    drop(pty);

    let sink = sink.lock();
    assert!(
        sink.fed > MAX_PENDING_READ_BYTES,
        "the flood must really have flowed: {} bytes",
        sink.fed
    );
    assert!(
        sink.worst_hold <= MAX_LOCKED_BYTES,
        "hold {}",
        sink.worst_hold
    );
    assert!(count >= MIN_FRAMES, "only {count} frames were read");
    assert!(worst < MAX_FRAME_WAIT, "a frame read waited {worst:?}");
    eprintln!(
        "frames {count}, worst wait {worst:?}, fed {} bytes",
        sink.fed
    );
}
