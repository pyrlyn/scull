//! The handle a terminal owns: spawn the child on a PTY, start the four threads,
//! take input and resize, shut everything down. Separate from the thread bodies
//! because this is the only module that touches the PTY crate's spawn and master
//! APIs, which keeps a later swap of that crate to one file.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, native_pty_system};

use crate::error::PtyError;
use crate::io_loop;
use crate::limits::SHUTDOWN_GRACE;
use crate::shared::{FinishGuard, Shared, Worker};
use crate::sink::Sink;
use crate::workers::{read_loop, wait_loop, write_loop};

/// Window size in cells, with the cell size in pixels where the system uses it
/// (`TIOCGWINSZ` pixel fields; ConPTY ignores them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    /// Lines of text.
    pub rows: u16,
    /// Columns of text.
    pub cols: u16,
    /// Width of the whole window in pixels, or 0.
    pub pixel_width: u16,
    /// Height of the whole window in pixels, or 0.
    pub pixel_height: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

/// What to run and where.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// The program, searched on `PATH`; `None` runs the user's shell.
    pub program: Option<OsString>,
    /// Arguments; ignored for the default shell.
    pub args: Vec<OsString>,
    /// Working directory; the parent's when `None`.
    pub cwd: Option<PathBuf>,
    /// Variables added to the inherited environment.
    pub env: Vec<(OsString, OsString)>,
    /// Initial window size.
    pub size: PtySize,
}

impl SpawnOptions {
    /// Run `program` with `args`.
    #[must_use]
    pub fn command(program: impl Into<OsString>, args: &[&str]) -> Self {
        Self {
            program: Some(program.into()),
            args: args.iter().map(OsString::from).collect(),
            ..Self::default()
        }
    }

    fn builder(&self) -> CommandBuilder {
        let mut builder = match &self.program {
            Some(program) => {
                let mut builder = CommandBuilder::new(program);
                builder.args(&self.args);
                builder
            }
            None => CommandBuilder::new_default_prog(),
        };
        if let Some(cwd) = &self.cwd {
            builder.cwd(cwd);
        }
        for (key, value) in &self.env {
            builder.env(key, value);
        }
        builder
    }
}

/// One child on one PTY, with its threads. Dropping it shuts everything down:
/// the child gets a hangup, the threads are joined, and no wakeup or sink call
/// happens afterwards. `child_exited` is not called for a shutdown.
pub struct Pty {
    shared: Arc<Shared>,
    /// Taken at shutdown: on Windows dropping it closes the pseudo console,
    /// which is what lets the reader see EOF.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    threads: Vec<(Worker, JoinHandle<()>)>,
}

impl Pty {
    /// Start the child and the threads that feed `sink`.
    ///
    /// `sink` is the lock the caller's frame reads also take; `wakeup` is called
    /// from the loop thread after each hold and after the exit, with no lock
    /// held, and must not block.
    ///
    /// # Errors
    ///
    /// When the PTY cannot be opened, the child cannot be started or a thread
    /// cannot be created; nothing is left running then.
    pub fn spawn<S: Sink>(
        options: &SpawnOptions,
        sink: Arc<Mutex<S>>,
        wakeup: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, PtyError> {
        let pair = native_pty_system()
            .openpty(options.size.into())
            .map_err(|error| PtyError::Open(format!("{error:#}")))?;
        let child = pair
            .slave
            .spawn_command(options.builder())
            .map_err(|error| PtyError::Spawn(format!("{error:#}")))?;
        // Our copy of the slave keeps the PTY open after the child is gone, so
        // the reader would never see EOF.
        drop(pair.slave);
        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| PtyError::Attach(format!("{error:#}")))?;
        let writer = master
            .take_writer()
            .map_err(|error| PtyError::Attach(format!("{error:#}")))?;

        let shared = Shared::new();
        let mut pty = Self {
            shared: Arc::clone(&shared),
            master: Mutex::new(Some(master)),
            killer: Mutex::new(child.clone_killer()),
            threads: Vec::new(),
        };
        pty.start(Worker::Reader, "scull-pty-read", {
            let shared = Arc::clone(&shared);
            move || read_loop(&shared, reader)
        })?;
        pty.start(Worker::Writer, "scull-pty-write", {
            let shared = Arc::clone(&shared);
            move || write_loop(&shared, writer)
        })?;
        pty.start(Worker::Waiter, "scull-pty-wait", {
            let shared = Arc::clone(&shared);
            move || wait_loop(&shared, child)
        })?;
        pty.start(Worker::Loop, "scull-pty-io", {
            let shared = Arc::clone(&shared);
            move || io_loop::run(&shared, &sink, &wakeup)
        })?;
        Ok(pty)
    }

    fn start(
        &mut self,
        worker: Worker,
        name: &'static str,
        body: impl FnOnce() + Send + 'static,
    ) -> Result<(), PtyError> {
        let guard = FinishGuard::new(&self.shared, worker);
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let _guard = guard;
                body();
            })
            .map_err(|source| PtyError::Thread { name, source })?;
        self.threads.push((worker, handle));
        Ok(())
    }

    /// Queue input for the child (keys, paste). Never blocks: returns how many
    /// leading bytes were accepted, fewer than `bytes.len()` when the write
    /// queue is full because the child is not reading; offer the rest later.
    ///
    /// # Errors
    ///
    /// [`PtyError::Closed`] once the terminal has shut down or the child no
    /// longer accepts input.
    pub fn write_input(&self, bytes: &[u8]) -> Result<usize, PtyError> {
        self.shared.push_input(bytes)
    }

    /// Tell the child the window changed size (`SIGWINCH` on Unix,
    /// `ResizePseudoConsole` on Windows).
    ///
    /// # Errors
    ///
    /// [`PtyError::Closed`] after shutdown, [`PtyError::Resize`] when the system
    /// refuses.
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        match self.master.lock().as_ref() {
            Some(master) => master
                .resize(size.into())
                .map_err(|error| PtyError::Resize(format!("{error:#}"))),
            None => Err(PtyError::Closed),
        }
    }

    /// Hold the child's output back, as during an interactive resize: the UI
    /// keeps drawing the last frame, the reader fills its capped queue and
    /// then blocks, and the child blocks in `write`. Resize once at the final
    /// size, then [`Self::resume_output`].
    pub fn pause_output(&self) {
        self.shared.set_paused(true);
    }

    /// Let output flow again after [`Self::pause_output`].
    pub fn resume_output(&self) {
        self.shared.set_paused(false);
    }

    /// Ask the child to end (`SIGHUP`, or termination on Windows). The exit
    /// still arrives through the sink.
    ///
    /// # Errors
    ///
    /// The operating system's refusal, for example when the child already ended.
    pub fn kill(&self) -> std::io::Result<()> {
        self.killer.lock().kill()
    }

    fn shutdown(&mut self) {
        self.shared.stop();
        // Already ended is the common case and not worth reporting.
        let _ = self.kill();
        drop(self.master.lock().take());
        let expected = self
            .threads
            .iter()
            .fold(0, |mask, (worker, _)| mask | worker.bit());
        let finished = self
            .shared
            .wait_settled(expected, Instant::now() + SHUTDOWN_GRACE);
        for (worker, handle) in self.threads.drain(..) {
            // A thread still in a blocking call (a child that ignores the
            // hangup and keeps the PTY open) is detached rather than waited for:
            // it ends by itself when the PTY does, and holds nothing of ours.
            if finished & worker.bit() != 0 {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shutdown();
    }
}
