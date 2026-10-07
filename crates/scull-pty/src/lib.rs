//! The pseudo-terminal and its threads. The only crate that spawns child
//! processes, so back-pressure and child exit live in one place.
//!
//! A [`Pty`] runs one child on a PTY (ConPTY on Windows) and moves its output
//! into a caller-supplied [`Sink`] behind a caller-supplied lock. The crate has
//! no workspace dependency: `scull-ffi` plugs the terminal state in as the sink.
//!
//! Four threads serve one terminal, because every call on the PTY crate's
//! handles blocks and none of them can be interrupted:
//!
//! - `scull-pty-read` reads the child's output into a bounded queue;
//! - `scull-pty-io` takes the sink lock, feeds a bounded batch, collects the
//!   sink's replies, releases the lock fairly and fires the wakeup;
//! - `scull-pty-write` writes replies and the user's input to the child;
//! - `scull-pty-wait` waits for the child and reports its status.
//!
//! The queues between them are the back-pressure: a busy lock or a stalled
//! sink fills the read queue, the reader blocks, the kernel buffer fills and the
//! child blocks in `write`. Nothing the child sends can grow memory past the
//! caps in [`limits`].

mod error;
mod io_loop;
mod pty;
mod shared;
mod sink;
mod workers;

pub mod limits;

pub use error::PtyError;
pub use parking_lot::Mutex;
pub use pty::{Pty, PtySize, SpawnOptions};
pub use sink::{ExitStatus, Sink};
