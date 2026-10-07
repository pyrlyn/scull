use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::tests::Scratch;
use crate::{Action, LiveConfig, Settings, Snapshot};

const PATIENCE: Duration = Duration::from_secs(20);

fn open(dir: &Scratch, name: &str) -> (LiveConfig, Arc<AtomicUsize>) {
    let wakeups = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakeups);
    let live = LiveConfig::open(dir.0.join(name), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    (live, wakeups)
}

/// Polls until `done` holds for the snapshot, as a host does between wakeups.
fn wait_for(live: &LiveConfig, what: &str, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let start = Instant::now();
    loop {
        let snapshot = live.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(start.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn a_missing_file_opens_as_the_defaults_and_is_watched() {
    let dir = Scratch::new("live-missing");
    let (live, wakeups) = open(&dir, "sub/config.toml");
    let snapshot = live.snapshot();
    assert_eq!(snapshot.generation, 1);
    assert_eq!(*snapshot.settings, Settings::default());
    assert!(snapshot.error.is_none() && snapshot.watching);
    assert_eq!(wakeups.load(Ordering::SeqCst), 0, "opening is not a change");
}

#[test]
fn editing_the_file_changes_the_settings_without_a_restart() {
    let dir = Scratch::new("live-edit");
    let (live, wakeups) = open(&dir, "config.toml");
    fs::write(
        live.path(),
        "[font]\nsize = 18\n[colors]\nscheme = \"scull-light\"\n",
    )
    .unwrap();
    let snapshot = wait_for(&live, "the new size", |s| s.settings.font.size == 18.0);
    assert_eq!(snapshot.generation, 2);
    assert_eq!(snapshot.settings.colors, crate::Scheme::ScullLight.theme());
    assert!(wakeups.load(Ordering::SeqCst) >= 1);

    // An editor's save: write elsewhere, rename over.
    let draft = dir.0.join("draft");
    fs::write(&draft, "scrollback = 42\n").unwrap();
    fs::rename(&draft, live.path()).unwrap();
    wait_for(&live, "the renamed file", |s| s.settings.scrollback == 42);

    fs::remove_file(live.path()).unwrap();
    let snapshot = wait_for(&live, "the defaults", |s| {
        *s.settings == Settings::default()
    });
    assert!(snapshot.error.is_none());
}

#[test]
fn a_broken_edit_keeps_the_last_good_settings_and_says_why() {
    let dir = Scratch::new("live-broken");
    let (live, _) = open(&dir, "config.toml");
    fs::write(live.path(), "scrollback = 7\n").unwrap();
    wait_for(&live, "7", |s| s.settings.scrollback == 7);

    fs::write(live.path(), "scrollback = \"many\"\n").unwrap();
    let snapshot = wait_for(&live, "the error", |s| s.error.is_some());
    assert_eq!(snapshot.settings.scrollback, 7);
    assert!(snapshot.error.is_some_and(|e| e.contains("config.toml")));

    fs::write(live.path(), "scrollback = 8\n").unwrap();
    let snapshot = wait_for(&live, "the fix", |s| s.error.is_none());
    assert_eq!(snapshot.settings.scrollback, 8);
}

#[test]
fn a_file_that_is_bad_at_startup_is_an_error_with_the_defaults() {
    let dir = Scratch::new("live-bad-start");
    fs::write(dir.0.join("config.toml"), "bogus = 1\n").unwrap();
    let (live, _) = open(&dir, "config.toml");
    let snapshot = live.snapshot();
    assert_eq!(*snapshot.settings, Settings::default());
    assert!(snapshot.error.is_some_and(|e| e.contains("unknown field")));
}

#[test]
fn reloading_an_unchanged_file_raises_no_wakeup() {
    let dir = Scratch::new("live-same");
    fs::write(dir.0.join("config.toml"), "scrollback = 3\n").unwrap();
    let (live, wakeups) = open(&dir, "config.toml");
    live.reload();
    live.reload();
    assert_eq!(
        (live.snapshot().generation, wakeups.load(Ordering::SeqCst)),
        (1, 0)
    );
}

#[test]
fn set_edits_one_key_in_place_and_applies_it_at_once() {
    let dir = Scratch::new("live-set");
    let path = dir.0.join("config.toml");
    fs::write(
        &path,
        "# my terminal\nscrollback = 5 # short\n\n[font]\nsize = 12\n",
    )
    .unwrap();
    let (live, _) = open(&dir, "config.toml");
    live.set("font.size", Some("15.5")).unwrap();
    live.set("font.family", Some("Menlo")).unwrap();
    live.set("colors.scheme", Some("solarized-dark")).unwrap();
    let snapshot = live.snapshot();
    assert_eq!(snapshot.settings.font.size, 15.5);
    assert_eq!(snapshot.settings.font.family, "Menlo");
    let text = read(&path);
    assert!(
        text.contains("# my terminal") && text.contains("scrollback = 5 # short"),
        "{text}"
    );
    assert!(
        text.contains("size = 15.5") && text.contains("[colors]"),
        "{text}"
    );

    live.set("font.family", None).unwrap();
    live.set("colors.foreground", Some("#00ff00")).unwrap();
    live.set("colors.foreground", None).unwrap();
    assert_eq!(live.snapshot().settings.font.family, "");
    assert!(!read(&path).contains("foreground") && !read(&path).contains("Menlo"));
}

#[test]
fn set_refuses_what_would_make_the_file_invalid_and_writes_nothing() {
    let dir = Scratch::new("live-set-bad");
    let path = dir.0.join("config.toml");
    fs::write(&path, "scrollback = 5\n").unwrap();
    let (live, _) = open(&dir, "config.toml");
    for (key, value) in [
        ("font.size", "999"),
        ("font.size", "big"),
        ("font.size", "nan"),
        ("scrollback", "-1"),
        ("scrollback", "2000000"),
        ("colors.scheme", "neon"),
        ("colors.cursor", "red"),
        ("font.nope", "1"),
        ("keybind", "x"),
        ("", "x"),
    ] {
        assert!(live.set(key, Some(value)).is_err(), "{key}={value}");
    }
    assert_eq!(read(&path), "scrollback = 5\n");
    assert_eq!(live.snapshot().generation, 1);
}

#[test]
fn set_creates_the_file_and_directory_and_leaves_no_temporary_file() {
    let dir = Scratch::new("live-set-new");
    let (live, _) = open(&dir, "a/b/config.toml");
    live.set("scrollback", Some("9")).unwrap();
    assert_eq!(read(live.path()), "scrollback = 9\n");
    let names = fs::read_dir(live.path().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, ["config.toml"]);
}

#[test]
fn set_does_not_edit_a_file_that_is_already_broken() {
    let dir = Scratch::new("live-set-broken");
    let path = dir.0.join("config.toml");
    fs::write(&path, "[font\n").unwrap();
    let (live, _) = open(&dir, "config.toml");
    assert!(live.set("scrollback", Some("9")).is_err());
    assert_eq!(read(&path), "[font\n");
}

#[cfg(unix)]
#[test]
fn set_writes_through_a_symlink_instead_of_replacing_it() {
    let dir = Scratch::new("live-set-link");
    let real = dir.0.join("real.toml");
    fs::write(&real, "scrollback = 1\n").unwrap();
    std::os::unix::fs::symlink(&real, dir.0.join("config.toml")).unwrap();
    let (live, _) = open(&dir, "config.toml");
    live.set("scrollback", Some("2")).unwrap();
    assert!(fs::symlink_metadata(live.path()).unwrap().is_symlink());
    assert_eq!(read(&real), "scrollback = 2\n");
}

#[test]
fn keybinds_survive_a_live_reload() {
    let dir = Scratch::new("live-keys");
    let (live, _) = open(&dir, "config.toml");
    fs::write(
        live.path(),
        "[[keybind]]\nkey = \"super+v\"\naction = \"none\"\n",
    )
    .unwrap();
    let snapshot = wait_for(&live, "the unbound paste", |s| {
        s.settings.bindings.len() == 8
    });
    assert!(
        snapshot
            .settings
            .bindings
            .iter()
            .all(|b| b.action != Action::Paste)
    );
}

#[test]
fn dropping_stops_the_watcher_for_good() {
    let dir = Scratch::new("live-drop");
    let (live, wakeups) = open(&dir, "config.toml");
    let path = live.path().to_owned();
    drop(live);
    fs::write(path, "scrollback = 1\n").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(wakeups.load(Ordering::SeqCst), 0);
}
