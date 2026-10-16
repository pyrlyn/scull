use std::ffi::OsString;
use std::path::PathBuf;

/// Where the config file lives when the host names no other: `scull/config.toml`
/// under `%APPDATA%` on Windows, otherwise under `$XDG_CONFIG_HOME` or
/// `~/.config`. macOS uses `~/.config` too, where terminal users look for it.
/// `None` when the environment has no home to build it from.
pub fn default_path() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name);
    default_path_in(var("XDG_CONFIG_HOME"), var("HOME"), var("APPDATA"))
}

fn default_path_in(
    xdg: Option<OsString>,
    home: Option<OsString>,
    appdata: Option<OsString>,
) -> Option<PathBuf> {
    // A relative base would put the file wherever the program happens to run.
    let absolute = |v: Option<OsString>| v.map(PathBuf::from).filter(|p| p.is_absolute());
    let base = if cfg!(windows) {
        absolute(appdata)?
    } else {
        absolute(xdg).or_else(|| absolute(home).map(|h| h.join(".config")))?
    };
    Some(base.join("scull").join("config.toml"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn path(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
        default_path_in(xdg.map(Into::into), home.map(Into::into), None)
    }

    #[test]
    fn xdg_wins_over_home_and_both_must_be_absolute() {
        let file = |dir: &str| Some(PathBuf::from(dir).join("scull/config.toml"));
        assert_eq!(path(Some("/x"), Some("/h")), file("/x"));
        assert_eq!(path(None, Some("/h")), file("/h/.config"));
        assert_eq!(path(Some("rel"), Some("/h")), file("/h/.config"));
        assert_eq!(path(Some("rel"), Some("rel")), None);
        assert_eq!(path(None, None), None);
    }
}
