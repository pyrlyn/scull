//! `[[keybind]]` entries: a chord such as `super+shift+=` and the action it
//! runs. Chords are in the vocabulary of `scull-input`, the same keys and
//! modifier bits a host puts in a key event, so a host matches a binding
//! with one comparison and no layout knowledge.

use std::fmt;

use scull_input::{Key, Modifiers};
use serde::{Deserialize, Serialize};

/// The most `[[keybind]]` entries a file may have.
pub const MAX_KEYBINDS: usize = 256;
/// The highest function key a chord may name.
const LAST_F_KEY: u8 = 35;
/// Modifiers that make a printable key a shortcut rather than typing.
const SHORTCUT_MODS: Modifiers = Modifiers::ALT
    .union(Modifiers::CTRL)
    .union(Modifiers::SUPER);

/// What a binding does. The numbers are the C ABI's `TT_ACTION_*` values and
/// never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum Action {
    /// Removes a default binding of the same chord; never in [`crate::Settings::bindings`].
    None = 0,
    /// Paste the clipboard.
    Paste = 1,
    /// Make the font one point larger.
    FontLarger = 2,
    /// Make the font one point smaller.
    FontSmaller = 3,
    /// Back to the configured font size.
    FontReset = 4,
    /// Scroll the history one screen up.
    ScrollPageUp = 5,
    /// Scroll the history one screen down.
    ScrollPageDown = 6,
    /// Scroll to the oldest history row.
    ScrollToTop = 7,
    /// Scroll back to the screen.
    ScrollToBottom = 8,
}

/// A key with the modifiers held, such as Command and `=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The key, by position on a US layout.
    pub key: Key,
    /// Shift, Alt, Control and Super; no other bit.
    pub mods: Modifiers,
}

/// A binding that is in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    /// What was pressed.
    pub chord: Chord,
    /// What it does; never [`Action::None`].
    pub action: Action,
}

const DEFAULTS: [(Key, Modifiers, Action); 9] = [
    (Key::Char('v'), Modifiers::SUPER, Action::Paste),
    (Key::Char('='), Modifiers::SUPER, Action::FontLarger),
    (
        Key::Char('='),
        Modifiers::SUPER.union(Modifiers::SHIFT),
        Action::FontLarger,
    ),
    (Key::Char('-'), Modifiers::SUPER, Action::FontSmaller),
    (Key::Char('0'), Modifiers::SUPER, Action::FontReset),
    (Key::PageUp, Modifiers::SHIFT, Action::ScrollPageUp),
    (Key::PageDown, Modifiers::SHIFT, Action::ScrollPageDown),
    (Key::Home, Modifiers::SUPER, Action::ScrollToTop),
    (Key::End, Modifiers::SUPER, Action::ScrollToBottom),
];

const KEY_NAMES: [(&str, Key); 15] = [
    ("space", Key::Char(' ')),
    ("escape", Key::Escape),
    ("enter", Key::Enter),
    ("tab", Key::Tab),
    ("backspace", Key::Backspace),
    ("insert", Key::Insert),
    ("delete", Key::Delete),
    ("left", Key::Left),
    ("right", Key::Right),
    ("up", Key::Up),
    ("down", Key::Down),
    ("page-up", Key::PageUp),
    ("page-down", Key::PageDown),
    ("home", Key::Home),
    ("end", Key::End),
];

/// Canonical spelling order; also the order [`Chord`]'s `Display` writes.
const MOD_NAMES: [(&str, Modifiers); 4] = [
    ("ctrl", Modifiers::CTRL),
    ("alt", Modifiers::ALT),
    ("shift", Modifiers::SHIFT),
    ("super", Modifiers::SUPER),
];

/// Spellings people use for the same modifiers.
const MOD_ALIASES: [(&str, Modifiers); 5] = [
    ("control", Modifiers::CTRL),
    ("option", Modifiers::ALT),
    ("opt", Modifiers::ALT),
    ("cmd", Modifiers::SUPER),
    ("win", Modifiers::SUPER),
];

fn modifier(token: &str) -> Option<Modifiers> {
    MOD_NAMES
        .iter()
        .chain(&MOD_ALIASES)
        .find_map(|&(name, bit)| (name == token).then_some(bit))
}

fn key(token: &str) -> Result<Key, String> {
    if let Some(&(_, key)) = KEY_NAMES.iter().find(|(name, _)| *name == token) {
        return Ok(key);
    }
    if let Some(n) = token.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=LAST_F_KEY).contains(&n)
    {
        return Ok(Key::F(n));
    }
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() && !c.is_whitespace() => Ok(Key::Char(c)),
        _ => Err(format!("`{token}` is not a key name")),
    }
}

impl TryFrom<String> for Chord {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let lowered = text.to_lowercase();
        let mut parts = lowered.split('+').collect::<Vec<_>>();
        let last = parts.pop().unwrap_or_default();
        let mut mods = Modifiers::empty();
        for token in parts {
            let bit = modifier(token)
                .ok_or_else(|| format!("`{token}` is not a modifier in `{text}`"))?;
            if mods.intersects(bit) {
                return Err(format!("`{text}` names {token} twice"));
            }
            mods |= bit;
        }
        let key = key(last).map_err(|e| format!("{e} in `{text}`"))?;
        // Plain typing must keep reaching the program.
        if matches!(key, Key::Char(_)) && !mods.intersects(SHORTCUT_MODS) {
            return Err(format!(
                "`{text}` would steal typing: a character key needs ctrl, alt or super"
            ));
        }
        Ok(Self { key, mods })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, bit) in MOD_NAMES {
            if self.mods.contains(bit) {
                write!(f, "{name}+")?;
            }
        }
        if let Some((name, _)) = KEY_NAMES.iter().find(|(_, k)| *k == self.key) {
            return f.write_str(name);
        }
        match self.key {
            Key::F(n) => write!(f, "f{n}"),
            Key::Char(c) => write!(f, "{c}"),
            // A key `key()` never builds; Debug keeps the output readable.
            other => write!(f, "{other:?}"),
        }
    }
}

impl From<Chord> for String {
    fn from(chord: Chord) -> Self {
        chord.to_string()
    }
}

// A chord is text in the file, so the schema says so; its grammar is the doc comment.
#[cfg(test)]
impl schemars::JsonSchema for Chord {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Chord".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Modifiers (ctrl, alt, shift, super) and one key joined by `+`: a character, \
                f1 to f35, or one of space, escape, enter, tab, backspace, insert, delete, left, right, \
                up, down, page-up, page-down, home, end. A character needs ctrl, alt or super.",
            "type": "string",
            "maxLength": 64,
        })
    }
}

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// One `[[keybind]]` table.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    /// The chord that triggers the action.
    key: Chord,
    /// The action; `none` frees a default chord.
    action: Action,
}

/// The `[[keybind]]` list, checked as a whole.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(try_from = "Vec<Entry>", into = "Vec<Entry>")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("maxItems" = 256), transform = crate::model::array_schema))]
pub(crate) struct Keybinds(Vec<Entry>);

impl TryFrom<Vec<Entry>> for Keybinds {
    type Error = String;

    fn try_from(entries: Vec<Entry>) -> Result<Self, String> {
        if entries.len() > MAX_KEYBINDS {
            return Err(format!(
                "at most {MAX_KEYBINDS} keybind entries, not {}",
                entries.len()
            ));
        }
        for (i, entry) in entries.iter().enumerate() {
            if entries[..i].iter().any(|earlier| earlier.key == entry.key) {
                return Err(format!("keybind `{}` appears twice", entry.key));
            }
        }
        Ok(Self(entries))
    }
}

impl From<Keybinds> for Vec<Entry> {
    fn from(keybinds: Keybinds) -> Self {
        keybinds.0
    }
}

impl Keybinds {
    /// The defaults with this list laid over them: the same chord replaces
    /// the default, `none` removes it.
    pub(crate) fn resolve(&self) -> Vec<Binding> {
        let mut bindings = DEFAULTS
            .iter()
            .map(|&(key, mods, action)| Binding {
                chord: Chord { key, mods },
                action,
            })
            .collect::<Vec<_>>();
        for entry in &self.0 {
            bindings.retain(|b| b.chord != entry.key);
            if entry.action != Action::None {
                bindings.push(Binding {
                    chord: entry.key,
                    action: entry.action,
                });
            }
        }
        bindings
    }
}
