//! Real children on a real PTY (ConPTY on Windows): output and exit status,
//! input, resize, replies and shutdown. Process-spawning tests live here, apart
//! from the unit tests, because they need the operating system's shell.

mod common;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use scull_pty::limits::SHUTDOWN_GRACE;
use scull_pty::{ExitStatus, Mutex, Pty, PtyError, PtySize, Sink, SpawnOptions};

/// A child that is slow to start on a loaded machine still finishes well inside this.
const GENEROUS: Duration = Duration::from_secs(30);

const EXIT_CODE: u32 = 3;

#[derive(Default)]
struct Recorder {
    output: Vec<u8>,
    exit: Option<ExitStatus>,
    /// Bytes fed before `child_exited`: the exit must come last.
    fed_after_exit: bool,
    /// Sent once, as soon as `trigger` appears in the output.
    reply: Vec<u8>,
    trigger: &'static str,
    cursor: common::CursorReport,
}

impl Recorder {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl Sink for Recorder {
    fn feed(&mut self, bytes: &[u8]) {
        self.fed_after_exit |= self.exit.is_some();
        self.output.extend_from_slice(bytes);
        self.cursor.feed(bytes);
    }

    fn drain_replies(&mut self, out: &mut Vec<u8>, limit: usize) {
        let limit = limit - self.cursor.drain(out, limit);
        if !self.trigger.is_empty() && self.text().contains(self.trigger) {
            let take = self.reply.len().min(limit);
            out.extend(self.reply.drain(..take));
        }
    }

    fn child_exited(&mut self, status: ExitStatus) {
        self.exit = Some(status);
    }
}

struct Session {
    pty: Pty,
    recorder: Arc<Mutex<Recorder>>,
    wakeups: Receiver<()>,
}

impl Session {
    fn start(options: &SpawnOptions, recorder: Recorder) -> Result<Self, PtyError> {
        let recorder = Arc::new(Mutex::new(recorder));
        let (tx, wakeups): (Sender<()>, Receiver<()>) = channel();
        let pty = Pty::spawn(options, Arc::clone(&recorder), move || {
            let _ = tx.send(());
        })?;
        Ok(Self {
            pty,
            recorder,
            wakeups,
        })
    }

    /// Block on wakeups, not on a clock, until `done` holds.
    fn wait_for(&self, what: &str, done: impl Fn(&Recorder) -> bool) {
        let deadline = Instant::now() + GENEROUS;
        while !done(&self.recorder.lock()) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                self.wakeups.recv_timeout(left).is_ok(),
                "timed out waiting for {what}; output so far: {:?}",
                self.recorder.lock().text()
            );
        }
    }
}

#[cfg(unix)]
fn echo_then_exit() -> SpawnOptions {
    SpawnOptions::command("/bin/sh", &["-c", "echo hi; exit 3"])
}

#[cfg(windows)]
fn echo_then_exit() -> SpawnOptions {
    SpawnOptions::command("cmd", &["/C", "echo hi & exit 3"])
}

#[test]
fn a_shell_delivers_its_output_and_then_its_exit_status() {
    let session = Session::start(&echo_then_exit(), Recorder::default()).unwrap();
    session.wait_for("the exit", |r| r.exit.is_some());
    let recorder = session.recorder.lock();
    assert!(recorder.text().contains("hi"), "{:?}", recorder.text());
    assert_eq!(recorder.exit.as_ref().map(|s| s.code), Some(EXIT_CODE));
    assert!(!recorder.fed_after_exit);
}

#[test]
fn a_missing_program_is_an_error_not_a_dead_terminal() {
    let options = SpawnOptions::command("scull-no-such-program-anywhere", &[]);
    let result = Session::start(&options, Recorder::default());
    assert!(matches!(result, Err(PtyError::Spawn(_))));
}

#[test]
fn dropping_the_handle_ends_a_running_child_within_the_shutdown_grace() {
    #[cfg(unix)]
    let options = SpawnOptions::command("sleep", &["600"]);
    #[cfg(windows)]
    let options = SpawnOptions::command("cmd", &["/C", "ping -n 600 127.0.0.1"]);
    let session = Session::start(&options, Recorder::default()).unwrap();
    let started = Instant::now();
    let recorder = Arc::clone(&session.recorder);
    drop(session);
    // Grace for the threads, plus slack for a loaded machine.
    assert!(
        started.elapsed() < SHUTDOWN_GRACE * 4,
        "{:?}",
        started.elapsed()
    );
    assert!(
        recorder.lock().exit.is_none(),
        "shutdown is not an exit event"
    );
}

#[test]
fn resizing_a_live_terminal_succeeds() {
    #[cfg(unix)]
    let options = SpawnOptions::command("sleep", &["600"]);
    #[cfg(windows)]
    let options = SpawnOptions::command("cmd", &["/C", "ping -n 600 127.0.0.1"]);
    let session = Session::start(&options, Recorder::default()).unwrap();
    let size = PtySize {
        rows: 40,
        cols: 120,
        ..PtySize::default()
    };
    assert!(session.pty.resize(size).is_ok());
}

#[cfg(unix)]
mod unix {
    use super::*;

    const ROWS: u16 = 31;
    const COLS: u16 = 101;

    #[test]
    fn resize_is_visible_to_the_child_and_input_reaches_it() {
        // `read` holds the child until we have resized and sent a line, so the
        // size it prints cannot be the initial one.
        let options = SpawnOptions::command("sh", &["-c", "read line; stty size"]);
        let session = Session::start(&options, Recorder::default()).unwrap();
        session
            .pty
            .resize(PtySize {
                rows: ROWS,
                cols: COLS,
                ..PtySize::default()
            })
            .unwrap();
        assert_eq!(session.pty.write_input(b"go\n").unwrap(), 3);
        let expected = format!("{ROWS} {COLS}");
        session.wait_for("the size report", |r| r.text().contains(&expected));
        session.wait_for("the exit", |r| r.exit.is_some());
    }

    #[test]
    fn replies_from_the_sink_are_written_to_the_child_by_the_loop() {
        // The child asks, the sink answers from the same lock hold that saw the
        // question, and the child prints what it was told.
        let options = SpawnOptions::command(
            "sh",
            &["-c", "printf 'ask?'; read answer; echo got:$answer"],
        );
        let recorder = Recorder {
            trigger: "ask?",
            reply: b"yes\n".to_vec(),
            ..Recorder::default()
        };
        let session = Session::start(&options, recorder).unwrap();
        session.wait_for("the child to echo the reply", |r| {
            r.text().contains("got:yes")
        });
    }

    #[test]
    fn input_larger_than_the_write_queue_is_cut_at_the_cap() {
        // The child never reads, so only the kernel buffer drains the queue.
        let options = SpawnOptions::command("sleep", &["600"]);
        let session = Session::start(&options, Recorder::default()).unwrap();
        let cap = scull_pty::limits::MAX_PENDING_WRITE_BYTES;
        let flood = vec![b'a'; cap * 8];
        assert_eq!(session.pty.write_input(&flood).unwrap(), cap);
    }
}
