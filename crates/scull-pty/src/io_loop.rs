//! The loop that owns the sink lock: the only place child output meets terminal
//! state. Separate from the blocking workers so that no call that can block on
//! the child ever runs while the lock is held, and so the fairness rule (bounded
//! hold, fair release, wakeup outside the lock) is stated once.

use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use crate::shared::{Shared, Work};
use crate::sink::Sink;

/// Drive the sink until the child has exited and its output is delivered, or
/// the terminal stops. `wakeup` is called after each hold, never under the lock.
pub(crate) fn run<S: Sink>(
    shared: &Shared,
    sink: &Arc<Mutex<S>>,
    wakeup: &(dyn Fn() + Send + Sync),
) {
    let mut scratch = Vec::new();
    loop {
        match shared.wait_for_work() {
            Work::Stop => return,
            Work::Data => {
                // Wait for the lock first, take the output after: whatever the
                // reader queued while a frame read held the lock goes in one
                // hold (up to the budget) instead of one hold per read. The
                // wait never stops the reader, which keeps filling the queue up
                // to its cap, so this is the buffering the back-pressure needs.
                let mut guard = sink.try_lock().unwrap_or_else(|| sink.lock());
                for chunk in shared.take_batch() {
                    guard.feed(&chunk);
                }
                shared.queue_replies(&mut *guard, &mut scratch);
                // A plain unlock lets this thread take the lock again before a
                // waiting frame read wakes; the fair one hands it over.
                MutexGuard::unlock_fair(guard);
                wakeup();
            }
            Work::Exited(status) => {
                let mut guard = sink.lock();
                guard.child_exited(status);
                MutexGuard::unlock_fair(guard);
                wakeup();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::limits::MAX_LOCKED_BYTES;
    use crate::sink::ExitStatus;

    #[derive(Default)]
    struct Log {
        fed: Vec<u8>,
        since_drain: usize,
        worst_hold: usize,
        replies_to_send: Vec<u8>,
        exit: Option<ExitStatus>,
        exit_after_all_output: bool,
    }

    impl Sink for Log {
        fn feed(&mut self, bytes: &[u8]) {
            self.fed.extend_from_slice(bytes);
            self.since_drain += bytes.len();
        }
        fn drain_replies(&mut self, out: &mut Vec<u8>, limit: usize) {
            // drain_replies ends every hold, so this is bytes per hold.
            self.worst_hold = self.worst_hold.max(self.since_drain);
            self.since_drain = 0;
            let take = self.replies_to_send.len().min(limit);
            out.extend(self.replies_to_send.drain(..take));
        }
        fn child_exited(&mut self, status: ExitStatus) {
            self.exit_after_all_output = !self.fed.is_empty();
            self.exit = Some(status);
        }
    }

    #[test]
    fn every_hold_feeds_at_most_the_budget_and_the_exit_comes_last() {
        let shared = Shared::new();
        let sink = Arc::new(Mutex::new(Log::default()));
        let chunks = 20;
        for _ in 0..chunks {
            assert!(shared.push_chunk(vec![b'x'; MAX_LOCKED_BYTES / 2 + 1]));
        }
        shared.reader_done();
        shared.set_exit(ExitStatus::unknown());
        let wakeups = AtomicUsize::new(0);
        run(&shared, &sink, &|| {
            wakeups.fetch_add(1, Ordering::SeqCst);
        });
        let log = sink.lock();
        assert!(
            log.worst_hold <= MAX_LOCKED_BYTES,
            "hold {}",
            log.worst_hold
        );
        assert_eq!(log.fed.len(), chunks * (MAX_LOCKED_BYTES / 2 + 1));
        assert_eq!(log.exit, Some(ExitStatus::unknown()));
        assert!(log.exit_after_all_output);
        assert!(wakeups.load(Ordering::SeqCst) > 1, "one wakeup per hold");
    }

    #[test]
    fn replies_collected_in_a_hold_reach_the_write_queue() {
        let shared = Shared::new();
        let sink = Arc::new(Mutex::new(Log {
            replies_to_send: b"\x1b[?62c".to_vec(),
            ..Log::default()
        }));
        assert!(shared.push_chunk(b"\x1b[c".to_vec()));
        shared.reader_done();
        shared.set_exit(ExitStatus::unknown());
        run(&shared, &sink, &|| {});
        let queued = shared.next_output();
        assert_eq!(queued.as_deref(), Some(&b"\x1b[?62c"[..]));
    }

    struct Panicky;

    impl Sink for Panicky {
        fn feed(&mut self, _bytes: &[u8]) {
            panic!("sink bug");
        }
        fn drain_replies(&mut self, _out: &mut Vec<u8>, _limit: usize) {}
        fn child_exited(&mut self, _status: ExitStatus) {}
    }

    #[test]
    fn a_panic_in_the_sink_stops_the_terminal_so_the_reader_cannot_block_forever() {
        let shared = Shared::new();
        let sink = Arc::new(Mutex::new(Panicky));
        assert!(shared.push_chunk(b"boom".to_vec()));
        let looper = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let _guard = crate::shared::FinishGuard::new(&shared, crate::shared::Worker::Loop);
                run(&shared, &sink, &|| {});
            })
        };
        assert!(looper.join().is_err(), "the panic is the point");
        assert!(shared.is_stopped());
        assert!(!shared.push_chunk(vec![0]), "a blocked reader is released");
    }
}
