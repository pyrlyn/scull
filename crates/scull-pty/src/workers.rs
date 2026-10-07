//! The three threads that sit in a blocking call on the PTY crate's handles:
//! the reader, the writer and the child waiter. They hold no terminal state and
//! take no sink lock, so a stalled sink can never stop them from draining the
//! kernel buffer up to the cap, and the loop never blocks on the child.

use std::io::{self, Read, Write};

use portable_pty::Child;

use crate::shared::Shared;
use crate::sink::ExitStatus;

/// Read the child's output into the bounded queue until EOF, a read error or
/// stop. An error is treated as EOF: Linux reports a hung-up PTY as `EIO`.
pub(crate) fn read_loop(shared: &Shared, mut reader: impl Read) {
    let mut buf = vec![0; crate::limits::READ_CHUNK_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(len) => {
                // push_chunk blocks while the queue is full: this is where a
                // flooding child is slowed down.
                if !shared.push_chunk(buf[..len].to_vec()) {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    shared.reader_done();
}

/// Write queued replies and input to the child, one bounded chunk at a time. A
/// failed write means the child no longer reads; the queue is dropped then.
pub(crate) fn write_loop(shared: &Shared, mut writer: impl Write) {
    while let Some(chunk) = shared.next_output() {
        if writer
            .write_all(&chunk)
            .and_then(|()| writer.flush())
            .is_err()
        {
            shared.writer_failed();
            return;
        }
        shared.output_written(chunk.len());
    }
}

/// Wait for the child and hand its status to the loop, which delivers it after
/// the output. The status goes through the loop, not around it, so exit and
/// output are ordered.
pub(crate) fn wait_loop(shared: &Shared, mut child: Box<dyn Child + Send + Sync>) {
    let status = child
        .wait()
        .map_or_else(|_| ExitStatus::unknown(), ExitStatus::from);
    shared.set_exit(status);
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::limits::{MAX_PENDING_READ_BYTES, WRITE_CHUNK_BYTES};
    use crate::shared::Work;

    const GENEROUS: Duration = Duration::from_secs(10);

    /// A child that never stops talking.
    struct Endless;

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(b'y');
            Ok(buf.len())
        }
    }

    #[test]
    fn a_flooding_reader_stops_at_the_read_cap_and_resumes_when_drained() {
        let shared = Shared::new();
        let reader = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || read_loop(&shared, Endless))
        };
        // Drain until the loop has taken more than the cap in total: the
        // reader must have been blocked and released at least once.
        let mut drained = 0;
        let deadline = Instant::now() + GENEROUS;
        while drained <= MAX_PENDING_READ_BYTES * 2 && Instant::now() < deadline {
            if shared.wait_for_work() == Work::Data {
                drained += shared.take_batch().iter().map(Vec::len).sum::<usize>();
            }
        }
        assert!(drained > MAX_PENDING_READ_BYTES * 2);
        shared.stop();
        assert!(reader.join().is_ok());
    }

    #[test]
    fn eof_marks_the_reader_done_after_the_last_chunk_is_queued() {
        let shared = Shared::new();
        read_loop(&shared, Cursor::new(b"abc".to_vec()));
        assert_eq!(shared.wait_for_work(), Work::Data);
        assert_eq!(shared.take_batch(), vec![b"abc".to_vec()]);
        shared.set_exit(ExitStatus::unknown());
        assert_eq!(shared.wait_for_work(), Work::Exited(ExitStatus::unknown()));
    }

    /// Records writes; fails after `ok_writes`.
    struct Recorder {
        written: Arc<parking_lot::Mutex<Vec<u8>>>,
        ok_writes: usize,
    }

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.ok_writes == 0 {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.ok_writes -= 1;
            self.written.lock().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_writer_sends_queued_input_in_bounded_chunks_and_in_order() {
        let shared = Shared::new();
        let written = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let input: Vec<u8> = (0..WRITE_CHUNK_BYTES * 3 + 5)
            .map(|i| u8::try_from(i % 251).unwrap_or(0))
            .collect();
        assert!(matches!(shared.push_input(&input), Ok(n) if n == input.len()));
        let writer = {
            let shared = Arc::clone(&shared);
            let recorder = Recorder {
                written: Arc::clone(&written),
                ok_writes: usize::MAX,
            };
            thread::spawn(move || write_loop(&shared, recorder))
        };
        let deadline = Instant::now() + GENEROUS;
        while written.lock().len() < input.len() && Instant::now() < deadline {
            thread::yield_now();
        }
        shared.stop();
        assert!(writer.join().is_ok());
        assert_eq!(*written.lock(), input);
    }

    #[test]
    fn a_failed_write_drops_the_queue_and_refuses_more_input() {
        let shared = Shared::new();
        assert!(matches!(shared.push_input(b"hello"), Ok(5)));
        let recorder = Recorder {
            written: Arc::default(),
            ok_writes: 0,
        };
        write_loop(&shared, recorder);
        assert!(shared.push_input(b"x").is_err());
    }
}
