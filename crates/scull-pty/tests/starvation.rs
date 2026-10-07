//! A child that never stops writing cannot starve a frame read. One thread plays
//! the UI, taking the shared lock the way a frame update does, while a flooding
//! child keeps the reader busy. Separate from `shell.rs` because it is a timing
//! test with its own load and bounds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use scull_pty::limits::{MAX_LOCKED_BYTES, MAX_PENDING_READ_BYTES};
use scull_pty::{ExitStatus, Mutex, Pty, Sink, SpawnOptions};

/// How long the child floods while frames are read.
const FLOOD_DURATION: Duration = Duration::from_secs(2);
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
}

impl Sink for Parsing {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        self.fed += bytes.len();
        self.since_drain += bytes.len();
    }

    fn drain_replies(&mut self, _out: &mut Vec<u8>, _limit: usize) {
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
    let sink = Arc::new(Mutex::new(Parsing::default()));
    let pty = Pty::spawn(&flooding_child(), Arc::clone(&sink), || {}).unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let frames = {
        let sink = Arc::clone(&sink);
        let running = Arc::clone(&running);
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
                thread::sleep(FRAME_INTERVAL);
            }
            (worst, count)
        })
    };

    thread::sleep(FLOOD_DURATION);
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
