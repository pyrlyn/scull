use std::path::{Path, PathBuf};

use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use serde_json::Value;

use super::*;
use crate::model::Config;

fn parse(text: &str) -> Result<Settings, ConfigError> {
    Settings::parse(text, Path::new("scull.toml"))
}

fn message(text: &str) -> String {
    parse(text).unwrap_err().to_string()
}

/// A scratch directory per test, removed on drop.
pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("scull-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_empty_file_is_the_defaults() {
    let settings = parse("").unwrap();
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.font.family, "");
    assert_eq!(settings.font.size, 13.0);
    assert_eq!(settings.scrollback, 10_000);
    assert_eq!(settings.colors, Scheme::ScullDark.theme());
}

#[test]
fn every_section_is_read() {
    let settings = parse(
        r##"
scrollback = 500

[font]
family = "JetBrains Mono"
size = 15

[colors]
scheme = "solarized-dark"
foreground = "#ffffff"
cursor = "#FF0000"
ansi = ["#010203", "#040506"]
"##,
    )
    .unwrap();
    assert_eq!(settings.font.family, "JetBrains Mono");
    assert_eq!(settings.font.size, 15.0);
    assert_eq!(settings.scrollback, 500);
    let solarized = Scheme::SolarizedDark.theme();
    assert_eq!(settings.colors.foreground, Rgb::new(0xFFFFFF));
    assert_eq!(settings.colors.background, solarized.background);
    assert_eq!(settings.colors.cursor, Rgb::new(0xFF0000));
    assert_eq!(settings.colors.ansi[0], Rgb::new(0x010203));
    assert_eq!(settings.colors.ansi[1], Rgb::new(0x040506));
    assert_eq!(settings.colors.ansi[2..], solarized.ansi[2..]);
}

#[test]
fn schemes_differ_and_each_has_sixteen_colours() {
    let themes = [Scheme::ScullDark, Scheme::ScullLight, Scheme::SolarizedDark].map(Scheme::theme);
    assert_ne!(themes[0], themes[1]);
    assert_ne!(themes[1], themes[2]);
    assert_eq!(themes[0].ansi[1], Rgb::new(0xCD0000), "xterm red");
}

#[test]
fn bad_values_are_refused_with_the_file_name() {
    for (text, needle) in [
        ("[font]\nsize = 3.9", "font size"),
        ("[font]\nsize = 200.1", "font size"),
        ("[font]\nsize = 0", "font size"),
        ("[font]\nsize = -1", "font size"),
        ("[font]\nsize = \"big\"", "invalid type"),
        ("[font]\nfamily = \"a\\u0007b\"", "control characters"),
        ("scrollback = 1000001", "scrollback"),
        ("scrollback = -1", "scrollback"),
        ("scrollback = 1.5", "invalid type"),
        ("[colors]\nscheme = \"neon\"", "unknown variant"),
        ("[colors]\nforeground = \"red\"", "not a colour"),
        ("[colors]\nforeground = \"#12345\"", "not a colour"),
        ("[colors]\nforeground = \"#+12345\"", "not a colour"),
        ("[colors]\nforeground = \"#12345é\"", "not a colour"),
        ("[colors]\nforeground = 0", "invalid type"),
        ("[colors]\nansi = \"#000000\"", "invalid type"),
        ("[font\nsize = 1", "TOML"),
    ] {
        let message = message(text);
        assert!(message.starts_with("scull.toml: "), "{text}: {message}");
        assert!(message.contains(needle), "{text}: {message}");
    }
}

#[test]
fn unknown_keys_are_refused_so_a_typo_is_not_silent() {
    assert!(message("scrolback = 5").contains("unknown field"));
    assert!(message("[font]\nsise = 5").contains("unknown field"));
    assert!(message("[colour]\n").contains("unknown field"));
}

#[test]
fn too_many_palette_overrides_are_refused() {
    let seventeen = ["\"#000000\""; 17].join(", ");
    assert!(message(&format!("[colors]\nansi = [{seventeen}]")).contains("at most 16"));
    let sixteen = ["\"#000000\""; 16].join(", ");
    assert!(parse(&format!("[colors]\nansi = [{sixteen}]")).is_ok());
}

#[test]
fn the_longest_family_is_accepted_and_one_more_byte_is_not() {
    let ok = "x".repeat(MAX_FAMILY_BYTES);
    assert_eq!(
        parse(&format!("[font]\nfamily = \"{ok}\""))
            .unwrap()
            .font
            .family,
        ok
    );
    let long = "x".repeat(MAX_FAMILY_BYTES + 1);
    assert!(message(&format!("[font]\nfamily = \"{long}\"")).contains("at most"));
}

#[test]
fn the_size_range_is_inclusive() {
    assert_eq!(parse("[font]\nsize = 4").unwrap().font.size, MIN_FONT_SIZE);
    assert_eq!(
        parse("[font]\nsize = 200").unwrap().font.size,
        MAX_FONT_SIZE
    );
    assert_eq!(parse("scrollback = 0").unwrap().scrollback, 0);
    assert_eq!(
        parse("scrollback = 1000000").unwrap().scrollback,
        MAX_SCROLLBACK
    );
}

#[test]
fn nan_and_infinite_sizes_are_refused() {
    assert!(message("[font]\nsize = nan").contains("font size"));
    assert!(message("[font]\nsize = inf").contains("font size"));
}

#[test]
fn a_missing_file_is_the_defaults() {
    let dir = Scratch::new("missing");
    assert_eq!(
        Settings::load(&dir.0.join("none.toml")).unwrap(),
        Settings::default()
    );
}

#[test]
fn a_file_is_loaded_and_an_oversized_one_is_refused() {
    let dir = Scratch::new("load");
    let path = dir.0.join("config.toml");
    std::fs::write(&path, "scrollback = 7\n").unwrap();
    assert_eq!(Settings::load(&path).unwrap().scrollback, 7);

    let big = format!("# {}\n", "x".repeat(MAX_FILE_BYTES as usize));
    std::fs::write(&path, big).unwrap();
    assert!(matches!(
        Settings::load(&path),
        Err(ConfigError::TooLarge { .. })
    ));

    let at_limit = format!("#{}", "x".repeat(MAX_FILE_BYTES as usize - 1));
    std::fs::write(&path, at_limit).unwrap();
    assert!(Settings::load(&path).is_ok());
}

#[test]
fn binary_and_unreadable_files_are_errors_naming_the_file() {
    let dir = Scratch::new("binary");
    let path = dir.0.join("config.toml");
    std::fs::write(&path, [0xFF, 0xFE, 0x00]).unwrap();
    let err = Settings::load(&path).unwrap_err().to_string();
    assert!(
        err.contains("config.toml") && err.contains("UTF-8"),
        "{err}"
    );
    // A directory where the file should be opens, then fails to read.
    let err = Settings::load(&dir.0).unwrap_err();
    assert!(matches!(err, ConfigError::Io { .. }), "{err}");
}

/// TOML has no null: a missing key is how a file says `None`, so `Option<T>`
/// is described as plain `T`.
fn drop_null(schema: &mut schemars::Schema) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(types)) = object.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let only = types.remove(0);
            object.insert("type".into(), only);
        }
    }
    if let Some(Value::Array(variants)) = object.get_mut("anyOf") {
        variants.retain(|v| v.get("type").is_none_or(|t| t != "null"));
        if let [only] = variants.as_slice() {
            let only = only.clone();
            object.remove("anyOf");
            if let Value::Object(only) = only {
                object.extend(only);
            }
        }
    }
    if let Some(default) = object.get_mut("default") {
        strip_nulls(default);
        if default.is_null() {
            object.remove("default");
        }
    }
}

fn strip_nulls(value: &mut Value) {
    if let Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
        map.values_mut().for_each(strip_nulls);
    }
}

fn schema_json() -> String {
    let generator = SchemaSettings::draft2020_12()
        .with_transform(RecursiveTransform(drop_null as fn(&mut schemars::Schema)))
        .into_generator();
    let mut json =
        serde_json::to_string_pretty(&generator.into_root_schema_for::<Config>()).unwrap();
    json.push('\n');
    json
}

#[test]
fn the_committed_schema_matches_the_types() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/config.schema.json");
    let rendered = schema_json();
    if std::env::var_os("SCULL_BLESS").is_some() {
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == rendered,
        "docs/config.schema.json is stale: run `just config-schema`"
    );
}

#[test]
fn the_schema_carries_the_ranges_of_the_types() {
    let schema: Value = serde_json::from_str(&schema_json()).unwrap();
    let size = &schema["$defs"]["FontSize"];
    assert_eq!(
        (size["minimum"].as_f64(), size["maximum"].as_f64()),
        (Some(4.0), Some(200.0))
    );
    assert_eq!(schema["$defs"]["Scrollback"]["maximum"], 1_000_000);
    assert_eq!(schema["additionalProperties"], false);
    assert!(!schema_json().contains("null"));
}

fn chord(text: &str) -> Chord {
    Chord::try_from(text.to_owned()).unwrap()
}

fn binding(settings: &Settings, text: &str) -> Option<Action> {
    let chord = chord(text);
    settings
        .bindings
        .iter()
        .find(|b| b.chord == chord)
        .map(|b| b.action)
}

#[test]
fn chords_parse_in_any_case_and_spelling_and_print_canonically() {
    use scull_input::{Key, Modifiers};
    let c = chord("Cmd+Shift+=");
    assert_eq!(c.key, Key::Char('='));
    assert_eq!(c.mods, Modifiers::SUPER | Modifiers::SHIFT);
    assert_eq!(chord("option+control+a"), chord("ctrl+alt+a"));
    assert_eq!(chord("win+A"), chord("super+a"));
    for canonical in [
        "ctrl+alt+shift+super+x",
        "shift+super+=",
        "shift+page-up",
        "f11",
        "ctrl+f35",
        "super+space",
        "alt+enter",
        "super+é",
    ] {
        assert_eq!(chord(canonical).to_string(), canonical);
    }
}

#[test]
fn bad_chords_say_what_is_wrong() {
    for (text, needle) in [
        ("", "not a key name"),
        ("super+", "not a key name"),
        ("super++", "not a modifier"),
        ("hyper+a", "not a modifier"),
        ("super+super+a", "twice"),
        ("super+f36", "not a key name"),
        ("super+f0", "not a key name"),
        ("super+ab", "not a key name"),
        ("super+\u{7}", "not a key name"),
        ("a", "steal typing"),
        ("shift+a", "steal typing"),
        ("shift+=", "steal typing"),
        ("super+shift", "not a key name"),
    ] {
        let err = Chord::try_from(text.to_owned()).unwrap_err();
        assert!(err.contains(needle), "{text:?}: {err}");
    }
    // Named keys are fine alone, characters are not.
    assert!(Chord::try_from("f1".to_owned()).is_ok());
    assert!(Chord::try_from("shift+home".to_owned()).is_ok());
}

#[test]
fn the_defaults_are_bound_without_a_file() {
    let settings = Settings::default();
    assert_eq!(settings.bindings.len(), 9);
    assert_eq!(binding(&settings, "super+v"), Some(Action::Paste));
    assert_eq!(
        binding(&settings, "super+shift+="),
        Some(Action::FontLarger)
    );
    assert_eq!(
        binding(&settings, "shift+page-up"),
        Some(Action::ScrollPageUp)
    );
    assert!(settings.bindings.iter().all(|b| b.action != Action::None));
}

#[test]
fn a_user_binding_replaces_the_default_of_its_chord_and_none_frees_it() {
    let settings = parse(
        r#"
[[keybind]]
key = "super+v"
action = "scroll-to-top"

[[keybind]]
key = "super+0"
action = "none"

[[keybind]]
key = "ctrl+alt+f"
action = "font-larger"
"#,
    )
    .unwrap();
    assert_eq!(binding(&settings, "super+v"), Some(Action::ScrollToTop));
    assert_eq!(binding(&settings, "super+0"), None);
    assert_eq!(binding(&settings, "ctrl+alt+f"), Some(Action::FontLarger));
    assert_eq!(binding(&settings, "super+-"), Some(Action::FontSmaller));
    assert_eq!(settings.bindings.len(), 9);
}

#[test]
fn keybind_entries_are_validated_when_the_file_is_read() {
    let twice = "[[keybind]]\nkey = \"super+v\"\naction = \"paste\"\n";
    assert!(message(&format!("{twice}{twice}")).contains("appears twice"));
    // The same chord spelled two ways is still twice.
    let other = "[[keybind]]\nkey = \"cmd+V\"\naction = \"none\"\n";
    assert!(message(&format!("{twice}{other}")).contains("appears twice"));
    assert!(message("[[keybind]]\nkey = \"a\"\naction = \"paste\"").contains("steal typing"));
    assert!(
        message("[[keybind]]\nkey = \"super+v\"\naction = \"fly\"").contains("unknown variant")
    );
    assert!(message("[[keybind]]\nkey = \"super+v\"").contains("missing field"));
    assert!(
        message("[[keybind]]\nkey = \"super+v\"\naction = \"paste\"\nx = 1")
            .contains("unknown field")
    );
}

#[test]
fn the_keybind_count_is_capped() {
    let entry = |i: u32| {
        // Distinct chords: a character and a function key under each modifier set.
        let keys = (b'a'..=b'z').map(char::from).collect::<Vec<_>>();
        let mods = [
            "ctrl",
            "alt",
            "super",
            "ctrl+alt",
            "ctrl+super",
            "alt+super",
            "ctrl+alt+super",
            "ctrl+shift",
            "alt+shift",
            "super+shift",
        ];
        let key = keys[i as usize % 26];
        let m = mods[i as usize / 26 % mods.len()];
        format!("[[keybind]]\nkey = \"{m}+{key}\"\naction = \"paste\"\n")
    };
    let at_cap = (0..MAX_KEYBINDS as u32).map(entry).collect::<String>();
    assert!(parse(&at_cap).is_ok());
    let over = format!("{at_cap}[[keybind]]\nkey = \"f1\"\naction = \"paste\"\n");
    assert!(message(&over).contains("at most 256"));
}
