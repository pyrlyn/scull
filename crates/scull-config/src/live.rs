//! The configuration kept current while the program runs: the file is
//! watched, re-read after a change, and the result is held for the host to
//! poll. A file that stops being valid keeps the last good settings and
//! reports why, so a typo mid-edit never resets the terminal.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::{ConfigError, Settings, edit};

/// Editors save in several steps (truncate, write, rename); changes this
/// close together are one reload, so a half-written file is not read.
const QUIET: Duration = Duration::from_millis(60);

/// What a host polls: the settings in force, and why the file on disk is
/// not (yet) what they came from.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Starts at 1 and grows whenever the settings or the error change.
    pub generation: u64,
    /// The last settings that were valid.
    pub settings: Arc<Settings>,
    /// Why the file could not be used the last time it was read, if so.
    pub error: Option<String>,
    /// Whether file changes are being watched.
    pub watching: bool,
}

struct State {
    generation: u64,
    settings: Arc<Settings>,
    error: Option<String>,
}

struct Shared {
    path: PathBuf,
    state: Mutex<State>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // A panic while holding it leaves plain values, still consistent.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reload(&self) {
        let loaded = Settings::load(&self.path);
        let changed = {
            let mut state = self.state();
            match loaded {
                Ok(settings) => {
                    let same = *state.settings == settings && state.error.is_none();
                    if !same {
                        state.settings = Arc::new(settings);
                        state.error = None;
                    }
                    !same
                }
                Err(error) => {
                    let message = error.to_string();
                    let same = state.error.as_deref() == Some(&message);
                    state.error = Some(message);
                    !same
                }
            }
            .then(|| state.generation += 1)
            .is_some()
        };
        // Outside the lock: the callback may poll straight away.
        if changed {
            (self.on_change)();
        }
    }
}

/// The configuration file with a watcher on it. Dropping it stops the
/// watcher and joins its thread, so no callback runs afterwards.
pub struct LiveConfig {
    shared: Arc<Shared>,
    // Dropped first, which closes the channel and ends the thread.
    watcher: Option<RecommendedWatcher>,
    thread: Option<JoinHandle<()>>,
}

impl LiveConfig {
    /// Loads `path` and starts watching it; `on_change` runs on a core thread
    /// whenever [`LiveConfig::snapshot`] would now differ. Never fails: a file
    /// that cannot be used is an error in the snapshot with default settings,
    /// and a watcher that cannot start only means no live reload.
    pub fn open(path: PathBuf, on_change: impl Fn() + Send + Sync + 'static) -> Self {
        let (settings, error) = match Settings::load(&path) {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::default(), Some(e.to_string())),
        };
        let shared = Arc::new(Shared {
            path,
            state: Mutex::new(State {
                generation: 1,
                settings: Arc::new(settings),
                error,
            }),
            on_change: Box::new(on_change),
        });
        let (watcher, thread) = match watch(&shared) {
            Some((watcher, thread)) => (Some(watcher), Some(thread)),
            None => (None, None),
        };
        Self {
            shared,
            watcher,
            thread,
        }
    }

    /// The settings and error as of the last reload.
    pub fn snapshot(&self) -> Snapshot {
        let state = self.shared.state();
        Snapshot {
            generation: state.generation,
            settings: Arc::clone(&state.settings),
            error: state.error.clone(),
            watching: self.watcher.is_some(),
        }
    }

    /// Reads the file again now.
    pub fn reload(&self) {
        self.shared.reload();
    }

    /// Changes one setting in the file (see [`edit`]) and applies it at once;
    /// `None` removes the key so its default applies.
    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), ConfigError> {
        edit::set(&self.shared.path, key, value)?;
        self.shared.reload();
        Ok(())
    }

    /// The file this reads and writes.
    pub fn path(&self) -> &Path {
        &self.shared.path
    }
}

impl Drop for LiveConfig {
    fn drop(&mut self) {
        drop(self.watcher.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Watches the file's directory, not the file: editors replace a file by
/// renaming another over it, which a watch on the old file would lose.
fn watch(shared: &Arc<Shared>) -> Option<(RecommendedWatcher, JoinHandle<()>)> {
    let dir = shared.path.parent()?.to_owned();
    let name = shared.path.file_name()?.to_owned();
    // There is nothing to watch in a directory that does not exist yet, and
    // a first config file is usually written into a fresh one.
    std::fs::create_dir_all(&dir).ok()?;
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Reading the file raises access events on some systems; reacting to
        // them would reload forever.
        let relevant = match &event {
            Ok(e) => !e.kind.is_access() && e.paths.iter().any(|p| p.file_name() == Some(&name)),
            Err(_) => true,
        };
        if relevant {
            let _ = tx.send(());
        }
    })
    .ok()?;
    watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;
    let shared = Arc::clone(shared);
    let thread = thread::Builder::new()
        .name("scull-config".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                loop {
                    match rx.recv_timeout(QUIET) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                shared.reload();
            }
        })
        .ok()?;
    Some((watcher, thread))
}
