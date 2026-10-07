//! Every cap and timeout of the PTY layer in one place. The child is untrusted:
//! anything it can make us hold, or make us wait for, is bounded here and the
//! tests name these constants rather than repeating numbers.

use std::time::Duration;

/// Most bytes fed to the sink in one lock hold. The same bound as Alacritty's
/// `MAX_LOCKED_READ` (`u16::MAX`, `alacritty_terminal/src/event_loop.rs`) and
/// Warp's 64 KiB: long enough to amortise the lock, short enough that a frame
/// read waits for at most one parse of this size.
pub const MAX_LOCKED_BYTES: usize = 64 * 1024;

/// Size of one read from the PTY. Never above [`MAX_LOCKED_BYTES`], so a whole
/// chunk always fits one hold and no chunk has to be split.
pub const READ_CHUNK_BYTES: usize = MAX_LOCKED_BYTES;

/// Most unprocessed output held between the reader and the sink. When the lock
/// is busy the reader keeps buffering up to this, then blocks (back-pressure).
/// The same size as Alacritty's PTY read buffer.
pub const MAX_PENDING_READ_BYTES: usize = 1024 * 1024;

/// Most bytes waiting to be written to the child: replies from the sink and
/// input from the UI together. Input beyond it is refused, replies beyond it
/// stay in the sink's own capped queue.
pub const MAX_PENDING_WRITE_BYTES: usize = 256 * 1024;

/// Size of one write to the child. A PTY buffer is a few KiB; a larger write
/// would only block for longer, and the queue is released per chunk.
pub const WRITE_CHUNK_BYTES: usize = 4 * 1024;

/// How long after the child exits the loop keeps delivering output before it
/// reports the exit when no more arrives. Normally EOF comes at once; ConPTY
/// and an orphaned grandchild that holds the PTY open never send it.
pub const EXIT_IDLE: Duration = Duration::from_millis(100);

/// Hard end of the post-exit drain even if output keeps coming, so a
/// grandchild that floods the PTY cannot delay the exit report forever.
pub const EXIT_DRAIN_MAX: Duration = Duration::from_secs(1);

/// How long shutdown waits for the worker threads before it detaches the ones
/// that are stuck in a blocking call.
pub const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Exit code reported when the operating system cannot tell how the child ended.
pub const UNKNOWN_EXIT_CODE: u32 = u32::MAX;

const _: () = assert!(READ_CHUNK_BYTES <= MAX_LOCKED_BYTES);
const _: () = assert!(MAX_LOCKED_BYTES <= MAX_PENDING_READ_BYTES);
const _: () = assert!(WRITE_CHUNK_BYTES <= MAX_PENDING_WRITE_BYTES);
