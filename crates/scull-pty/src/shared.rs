//! The one mutex-protected state the four threads of a terminal share: the read
//! queue, the write queue, the child's exit and the stop flag. Separate from the
//! thread bodies so every cap lives, and is tested, in one place, and so the
//! lock order is trivial: this mutex is a leaf and is never held across a call
//! out (the sink lock may be held when it is taken, never the other way round).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Condvar, Mutex};

use crate::error::PtyError;
use crate::limits::{
    EXIT_DRAIN_MAX, EXIT_IDLE, MAX_LOCKED_BYTES, MAX_PENDING_READ_BYTES, MAX_PENDING_WRITE_BYTES,
    WRITE_CHUNK_BYTES,
};
use crate::sink::{ExitStatus, Sink};

/// The threads of one terminal, as bits of [`State::finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Worker {
    Reader,
    Loop,
    Writer,
    Waiter,
}

impl Worker {
    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::Reader => 1,
            Self::Loop => 2,
            Self::Writer => 4,
            Self::Waiter => 8,
        }
    }
}

/// What the I/O loop has to do next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Output is queued.
    Data,
    /// The child ended and its output is drained: report it and finish.
    Exited(ExitStatus),
    /// The terminal is shutting down.
    Stop,
    /// The sink's deadline passed with no output to feed.
    Tick,
}

struct State {
    chunks: VecDeque<Vec<u8>>,
    queued: usize,
    /// The reader hit EOF or a read error: no more output will come.
    reader_done: bool,
    exit: Option<ExitStatus>,
    exit_at: Option<Instant>,
    last_data: Instant,
    outbox: VecDeque<u8>,
    writer_failed: bool,
    stopped: bool,
    /// Output stays queued, as during an interactive resize: the reader
    /// fills the queue and then blocks, so the child blocks too.
    paused: bool,
    /// Bits of [`Worker`] whose thread has returned.
    finished: u8,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    /// The loop waits here for output, exit or stop.
    loop_wake: Condvar,
    /// The reader waits here for room in the read queue.
    read_space: Condvar,
    /// The writer waits here for bytes to write.
    write_wake: Condvar,
    /// Shutdown waits here for the threads to return.
    settled: Condvar,
}

impl Shared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                chunks: VecDeque::new(),
                queued: 0,
                reader_done: false,
                exit: None,
                exit_at: None,
                last_data: Instant::now(),
                outbox: VecDeque::new(),
                writer_failed: false,
                stopped: false,
                paused: false,
                finished: 0,
            }),
            loop_wake: Condvar::new(),
            read_space: Condvar::new(),
            write_wake: Condvar::new(),
            settled: Condvar::new(),
        })
    }

    // Reader side.

    /// Queue one read. Blocks while the queue is full (back-pressure) and
    /// returns false when the terminal stopped meanwhile.
    pub(crate) fn push_chunk(&self, chunk: Vec<u8>) -> bool {
        let mut state = self.state.lock();
        // Strict: the queue never holds more than the cap, so the total in
        // memory is the cap plus the one read the reader holds while it waits.
        while !state.stopped && state.queued + chunk.len() > MAX_PENDING_READ_BYTES {
            self.read_space.wait(&mut state);
        }
        if state.stopped {
            return false;
        }
        state.queued += chunk.len();
        state.chunks.push_back(chunk);
        state.last_data = Instant::now();
        self.loop_wake.notify_one();
        true
    }

    /// The reader reached EOF or failed: no more output.
    pub(crate) fn reader_done(&self) {
        self.state.lock().reader_done = true;
        self.loop_wake.notify_one();
    }

    // Waiter side.

    pub(crate) fn set_exit(&self, status: ExitStatus) {
        let mut state = self.state.lock();
        state.exit = Some(status);
        state.exit_at = Some(Instant::now());
        self.loop_wake.notify_one();
    }

    // I/O loop side.

    /// Block until there is something for the loop to do, or `deadline`
    /// (the sink's) passes. Output always comes before the exit, and the
    /// exit is held back until the output is drained: EOF, or quiet for
    /// [`EXIT_IDLE`], or [`EXIT_DRAIN_MAX`] after the exit. While paused,
    /// output and the exit behind it wait.
    pub(crate) fn wait_for_work(&self, deadline: Option<Instant>) -> Work {
        let mut state = self.state.lock();
        loop {
            if state.stopped {
                return Work::Stop;
            }
            if state.queued > 0 && !state.paused {
                return Work::Data;
            }
            let now = Instant::now();
            if deadline.is_some_and(|d| now >= d) {
                return Work::Tick;
            }
            let exit_by = match state.exit_at {
                Some(exit_at) if state.queued == 0 => {
                    let by =
                        (state.last_data.max(exit_at) + EXIT_IDLE).min(exit_at + EXIT_DRAIN_MAX);
                    if (state.reader_done || now >= by)
                        && let Some(status) = state.exit.clone()
                    {
                        return Work::Exited(status);
                    }
                    Some(by)
                }
                _ => None,
            };
            match exit_by.into_iter().chain(deadline).min() {
                Some(by) => {
                    self.loop_wake.wait_until(&mut state, by);
                }
                None => self.loop_wake.wait(&mut state),
            }
        }
    }

    /// Hold output back (`true`) or let it flow again.
    pub(crate) fn set_paused(&self, paused: bool) {
        self.state.lock().paused = paused;
        self.loop_wake.notify_one();
    }

    /// Take queued output for one lock hold: whole chunks, at most
    /// [`MAX_LOCKED_BYTES`] in all. Every chunk is at most that size, so the
    /// first always fits and none is ever split.
    pub(crate) fn take_batch(&self) -> Vec<Vec<u8>> {
        let mut state = self.state.lock();
        let mut batch = Vec::new();
        let mut taken = 0;
        while let Some(front) = state.chunks.front() {
            if taken + front.len() > MAX_LOCKED_BYTES {
                break;
            }
            taken += front.len();
            if let Some(chunk) = state.chunks.pop_front() {
                batch.push(chunk);
            }
        }
        state.queued -= taken;
        self.read_space.notify_one();
        batch
    }

    /// Move the sink's replies into the write queue, clamped to its room. The
    /// clamp is ours, not the sink's: whatever the sink does, the cap holds.
    pub(crate) fn queue_replies(&self, sink: &mut dyn Sink, scratch: &mut Vec<u8>) {
        let mut state = self.state.lock();
        if state.writer_failed {
            return;
        }
        let room = MAX_PENDING_WRITE_BYTES - state.outbox.len();
        if room == 0 {
            return;
        }
        scratch.clear();
        sink.drain_replies(scratch, room);
        scratch.truncate(room);
        if !scratch.is_empty() {
            state.outbox.extend(scratch.drain(..));
            self.write_wake.notify_one();
        }
    }

    // Writer side.

    /// Block until bytes are queued; copy out the next chunk to write. It stays
    /// queued, and counted against the cap, until [`Self::output_written`].
    pub(crate) fn next_output(&self) -> Option<Vec<u8>> {
        let mut state = self.state.lock();
        loop {
            if state.stopped {
                return None;
            }
            if !state.outbox.is_empty() {
                let len = state.outbox.len().min(WRITE_CHUNK_BYTES);
                return Some(state.outbox.iter().take(len).copied().collect());
            }
            self.write_wake.wait(&mut state);
        }
    }

    pub(crate) fn output_written(&self, len: usize) {
        let mut state = self.state.lock();
        let len = len.min(state.outbox.len());
        state.outbox.drain(..len);
    }

    /// The child no longer reads: drop what is queued and refuse more.
    pub(crate) fn writer_failed(&self) {
        let mut state = self.state.lock();
        state.writer_failed = true;
        state.outbox.clear();
    }

    // Caller side.

    /// Queue input for the child. Returns how many bytes were taken, which is
    /// fewer than offered when the queue is full; the caller keeps the rest.
    pub(crate) fn push_input(&self, bytes: &[u8]) -> Result<usize, PtyError> {
        let mut state = self.state.lock();
        if state.stopped || state.writer_failed {
            return Err(PtyError::Closed);
        }
        let take = bytes
            .len()
            .min(MAX_PENDING_WRITE_BYTES - state.outbox.len());
        state.outbox.extend(&bytes[..take]);
        if take > 0 {
            self.write_wake.notify_one();
        }
        Ok(take)
    }

    /// Stop everything: wake every waiter so each thread notices and returns.
    pub(crate) fn stop(&self) {
        self.state.lock().stopped = true;
        self.loop_wake.notify_all();
        self.read_space.notify_all();
        self.write_wake.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn is_stopped(&self) -> bool {
        self.state.lock().stopped
    }

    fn mark_finished(&self, worker: Worker) {
        self.state.lock().finished |= worker.bit();
        self.settled.notify_all();
    }

    /// Wait until every thread in `expected` (bits of [`Worker`]) has returned
    /// or `deadline` passes; returns the bits of the ones that did.
    pub(crate) fn wait_settled(&self, expected: u8, deadline: Instant) -> u8 {
        let mut state = self.state.lock();
        while state.finished & expected != expected {
            if self.settled.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
        state.finished
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        self.state.lock().queued
    }
}

/// Marks a thread as returned when dropped, also when it unwinds. The loop
/// ending for any reason, a panic in the sink included, stops the terminal, so
/// the reader cannot block forever on a queue nobody empties.
pub(crate) struct FinishGuard {
    shared: Arc<Shared>,
    worker: Worker,
}

impl FinishGuard {
    pub(crate) fn new(shared: &Arc<Shared>, worker: Worker) -> Self {
        Self {
            shared: Arc::clone(shared),
            worker,
        }
    }
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if self.worker == Worker::Loop {
            self.shared.stop();
        }
        self.shared.mark_finished(self.worker);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    /// Long enough for a thread that can proceed to have done so, short enough
    /// for the suite; only used to assert that something did NOT happen.
    const NEGATIVE_WAIT: Duration = Duration::from_millis(150);
    const GENEROUS: Duration = Duration::from_secs(10);
    const CHUNK: usize = 16 * 1024;

    fn fill_to_cap(shared: &Shared) {
        for _ in 0..MAX_PENDING_READ_BYTES / CHUNK {
            assert!(shared.push_chunk(vec![0; CHUNK]));
        }
        assert_eq!(shared.queued(), MAX_PENDING_READ_BYTES);
    }

    #[test]
    fn push_chunk_blocks_at_the_read_cap_until_the_loop_takes_a_batch() {
        let shared = Shared::new();
        fill_to_cap(&shared);

        let (done_tx, done_rx) = mpsc::channel();
        let pusher = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let accepted = shared.push_chunk(vec![0; CHUNK]);
                let _ = done_tx.send(accepted);
            })
        };
        assert!(done_rx.recv_timeout(NEGATIVE_WAIT).is_err(), "must block");
        assert_eq!(
            shared.queued(),
            MAX_PENDING_READ_BYTES,
            "cap never exceeded"
        );

        assert!(!shared.take_batch().is_empty());
        assert_eq!(done_rx.recv_timeout(GENEROUS), Ok(true));
        assert!(shared.queued() <= MAX_PENDING_READ_BYTES);
        assert!(pusher.join().is_ok());
    }

    #[test]
    fn stop_releases_a_reader_blocked_on_a_full_queue() {
        let shared = Shared::new();
        fill_to_cap(&shared);
        let (done_tx, done_rx) = mpsc::channel();
        let pusher = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let _ = done_tx.send(shared.push_chunk(vec![0; CHUNK]));
            })
        };
        assert!(done_rx.recv_timeout(NEGATIVE_WAIT).is_err());
        shared.stop();
        assert_eq!(done_rx.recv_timeout(GENEROUS), Ok(false));
        assert!(pusher.join().is_ok());
    }

    #[test]
    fn take_batch_never_exceeds_the_per_hold_budget_and_never_splits_a_chunk() {
        let shared = Shared::new();
        // Odd sizes on purpose: a bigger chunk follows a small one, and one
        // chunk is exactly the budget.
        let sizes = [
            MAX_LOCKED_BYTES * 2 / 3,
            MAX_LOCKED_BYTES / 2,
            1,
            MAX_LOCKED_BYTES,
            MAX_LOCKED_BYTES / 3,
            MAX_LOCKED_BYTES / 3,
            MAX_LOCKED_BYTES / 4,
        ];
        for size in sizes {
            assert!(shared.push_chunk(vec![7; size]));
        }
        let mut seen = Vec::new();
        loop {
            let batch = shared.take_batch();
            if batch.is_empty() {
                break;
            }
            let total: usize = batch.iter().map(Vec::len).sum();
            assert!(total <= MAX_LOCKED_BYTES, "hold of {total} bytes");
            seen.extend(batch.iter().map(Vec::len));
        }
        assert_eq!(seen, sizes, "order kept, no chunk split or merged");
        assert_eq!(shared.queued(), 0);
    }

    /// A sink that answers every hold with far more than it was allowed to.
    struct Chatty;

    impl Sink for Chatty {
        fn feed(&mut self, _bytes: &[u8]) {}
        fn drain_replies(&mut self, out: &mut Vec<u8>, _limit: usize) {
            out.resize(MAX_PENDING_WRITE_BYTES * 4, b'r');
        }
        fn child_exited(&mut self, _status: ExitStatus) {}
    }

    #[test]
    fn replies_are_clamped_to_the_write_queue_room_even_if_the_sink_ignores_the_limit() {
        let shared = Shared::new();
        let mut scratch = Vec::new();
        shared.queue_replies(&mut Chatty, &mut scratch);
        shared.queue_replies(&mut Chatty, &mut scratch);
        assert_eq!(shared.state.lock().outbox.len(), MAX_PENDING_WRITE_BYTES);
        assert!(matches!(shared.push_input(b"more"), Ok(0)));
    }

    #[test]
    fn push_input_takes_only_what_fits_and_reports_the_rest_as_refused() {
        let shared = Shared::new();
        let offered = vec![b'k'; MAX_PENDING_WRITE_BYTES + 10];
        assert!(matches!(shared.push_input(&offered), Ok(n) if n == MAX_PENDING_WRITE_BYTES));
        assert!(matches!(shared.push_input(b"x"), Ok(0)));
        // The writer frees room per chunk, not all at once.
        let chunk = shared.next_output();
        assert_eq!(chunk.as_ref().map(Vec::len), Some(WRITE_CHUNK_BYTES));
        shared.output_written(WRITE_CHUNK_BYTES);
        assert!(matches!(shared.push_input(&offered), Ok(n) if n == WRITE_CHUNK_BYTES));
    }

    #[test]
    fn push_input_after_a_write_failure_or_stop_is_closed() {
        let shared = Shared::new();
        shared.writer_failed();
        assert!(matches!(shared.push_input(b"x"), Err(PtyError::Closed)));
        let shared = Shared::new();
        shared.stop();
        assert!(matches!(shared.push_input(b"x"), Err(PtyError::Closed)));
    }

    #[test]
    fn output_queued_before_the_exit_is_delivered_before_the_exit_is_reported() {
        let shared = Shared::new();
        assert!(shared.push_chunk(b"last words".to_vec()));
        shared.set_exit(ExitStatus::unknown());
        assert_eq!(shared.wait_for_work(None), Work::Data);
        assert_eq!(shared.take_batch().len(), 1);
        shared.reader_done();
        assert_eq!(
            shared.wait_for_work(None),
            Work::Exited(ExitStatus::unknown())
        );
    }

    #[test]
    fn exit_is_reported_after_a_quiet_period_when_the_pty_never_reaches_eof() {
        let shared = Shared::new();
        let started = Instant::now();
        shared.set_exit(ExitStatus::unknown());
        assert_eq!(
            shared.wait_for_work(None),
            Work::Exited(ExitStatus::unknown())
        );
        assert!(started.elapsed() >= EXIT_IDLE);
        assert!(started.elapsed() < EXIT_DRAIN_MAX);
    }

    #[test]
    fn wait_for_work_returns_stop_once_stopped_even_with_output_queued() {
        let shared = Shared::new();
        assert!(shared.push_chunk(vec![1]));
        shared.stop();
        assert_eq!(shared.wait_for_work(None), Work::Stop);
    }

    #[test]
    fn paused_output_waits_and_the_sink_deadline_still_ticks() {
        let shared = Shared::new();
        assert!(shared.push_chunk(vec![1]));
        shared.set_paused(true);
        assert_eq!(shared.wait_for_work(Some(Instant::now())), Work::Tick);
        shared.set_paused(false);
        assert_eq!(shared.wait_for_work(None), Work::Data);
    }

    #[test]
    fn the_exit_waits_behind_paused_output() {
        let shared = Shared::new();
        assert!(shared.push_chunk(vec![1]));
        shared.reader_done();
        shared.set_exit(ExitStatus::unknown());
        shared.set_paused(true);
        let soon = Instant::now() + NEGATIVE_WAIT;
        assert_eq!(shared.wait_for_work(Some(soon)), Work::Tick);
        shared.set_paused(false);
        assert_eq!(shared.wait_for_work(None), Work::Data);
        shared.take_batch();
        assert_eq!(
            shared.wait_for_work(None),
            Work::Exited(ExitStatus::unknown())
        );
    }

    #[test]
    fn resuming_wakes_a_waiting_loop() {
        let shared = Shared::new();
        assert!(shared.push_chunk(vec![1]));
        shared.set_paused(true);
        let (tx, rx) = mpsc::channel();
        let waiter = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || tx.send(shared.wait_for_work(None)).unwrap())
        };
        assert!(
            rx.recv_timeout(NEGATIVE_WAIT).is_err(),
            "paused output waits"
        );
        shared.set_paused(false);
        assert_eq!(rx.recv_timeout(GENEROUS).unwrap(), Work::Data);
        waiter.join().unwrap();
    }

    #[test]
    fn finish_guard_marks_the_thread_and_a_loop_guard_also_stops_the_terminal() {
        let shared = Shared::new();
        drop(FinishGuard::new(&shared, Worker::Reader));
        assert!(!shared.is_stopped());
        drop(FinishGuard::new(&shared, Worker::Loop));
        assert!(shared.is_stopped());
        let expected = Worker::Reader.bit() | Worker::Loop.bit();
        let mask = shared.wait_settled(expected, Instant::now() + NEGATIVE_WAIT);
        assert_eq!(mask, expected);
    }
}
