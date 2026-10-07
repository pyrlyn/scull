//! The error type of the crate. Separate so every module can return it without
//! depending on the PTY handle.

use std::io;

/// Why a PTY operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PtyError {
    /// The system refused to create a pseudo-terminal.
    #[error("cannot open a pseudo-terminal: {0}")]
    Open(String),
    /// The child could not be started (missing program, bad directory, ...).
    #[error("cannot start the child process: {0}")]
    Spawn(String),
    /// The reader or writer end of the PTY could not be taken.
    #[error("cannot attach to the pseudo-terminal: {0}")]
    Attach(String),
    /// The window size could not be applied.
    #[error("cannot resize the pseudo-terminal: {0}")]
    Resize(String),
    /// A worker thread could not be started.
    #[error("cannot start the {name} thread: {source}")]
    Thread {
        /// Name of the thread that failed to start.
        name: &'static str,
        /// The operating system's reason.
        source: io::Error,
    },
    /// The terminal has shut down: the child is gone or the handle was closed.
    #[error("the pseudo-terminal is closed")]
    Closed,
}
