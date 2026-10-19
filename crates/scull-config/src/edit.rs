//! Changing one setting in the file on disk, for a settings view. The rest
//! of the file, comments included, stays as the user wrote it.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, value};

use crate::model::read_capped;
use crate::{ConfigError, Settings};

/// How a setting's text becomes a TOML value.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Float,
    Integer,
}

/// The settings a view may change: `section.key` or a top-level key. A
/// fixed list, so a view cannot write a key the schema does not have.
const SETTINGS: [(&str, Kind); 7] = [
    ("font.family", Kind::Text),
    ("font.size", Kind::Float),
    ("colors.scheme", Kind::Text),
    ("colors.foreground", Kind::Text),
    ("colors.background", Kind::Text),
    ("colors.cursor", Kind::Text),
    ("scrollback", Kind::Integer),
];

/// Sets `key` to `text` in the file at `path`, or removes it when `text` is
/// `None` so the default applies again. The result must still be a valid
/// file, or nothing is written.
pub(crate) fn set(path: &Path, key: &str, text: Option<&str>) -> Result<(), ConfigError> {
    let invalid = |message: String| ConfigError::Invalid {
        path: path.to_owned(),
        message,
    };
    let Some(&(_, kind)) = SETTINGS.iter().find(|(name, _)| *name == key) else {
        return Err(invalid(format!(
            "`{key}` is not a setting that can be changed"
        )));
    };
    let source = match read_capped(path) {
        Err(ConfigError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            String::new()
        }
        other => other?,
    };
    let mut doc = source
        .parse::<DocumentMut>()
        .map_err(|e| invalid(e.to_string()))?;
    let (section, name) = key
        .split_once('.')
        .map_or((None, key), |(s, n)| (Some(s), n));
    let table = match section {
        Some(section) => doc
            .entry(section)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| invalid(format!("`{section}` is not a table")))?,
        None => doc.as_table_mut(),
    };
    match text {
        Some(text) => {
            table.insert(name, value_of(kind, text).map_err(invalid)?);
        }
        None => {
            table.remove(name);
        }
    }
    let candidate = doc.to_string();
    Settings::parse(&candidate, path)?;
    write_atomically(path, &candidate)
}

fn value_of(kind: Kind, text: &str) -> Result<Item, String> {
    let number = |e: String| format!("`{text}` is not a number: {e}");
    Ok(match kind {
        Kind::Text => value(text),
        Kind::Float => match text.parse::<f64>() {
            Ok(n) if n.is_finite() => value(n),
            Ok(_) => return Err(format!("`{text}` is not a finite number")),
            Err(e) => return Err(number(e.to_string())),
        },
        Kind::Integer => value(i64::from(
            text.parse::<u32>().map_err(|e| number(e.to_string()))?,
        )),
    })
}

/// Replaces the file in one rename, so a watcher or a crash never sees half
/// of it. A symlinked file (dotfile managers) is written through, not replaced.
fn write_atomically(path: &Path, text: &str) -> Result<(), ConfigError> {
    let io = |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    };
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io(ErrorKind::InvalidInput.into()));
    };
    fs::create_dir_all(dir).map_err(io)?;
    let temp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = fs::write(&temp, text).and_then(|()| fs::rename(&temp, &target));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(io)
}
