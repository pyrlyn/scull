//! Generates `crates/scull-unicode/src/tables.rs` and its conformance fixture
//! from pinned Unicode data files. A separate binary so the library never
//! touches the network or the filesystem, and so the tables are reviewed as
//! committed source instead of being rebuilt on every compile.
//!
//! Usage: `scull-ucd-gen [--check]`. Missing data files are downloaded into
//! the gitignored `target/ucd/<version>/` cache; every file is verified
//! against its SHA-256 before use. `--check` fails if a committed output
//! differs from what the data files produce.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

const VERSION: (u8, u8, u8) = (18, 0, 0);
/// Data files under `https://www.unicode.org/Public/<version>/ucd/`, with the
/// SHA-256 of the released file (checked 2026-10-07).
const FILES: [(&str, &str); 7] = [
    (
        "EastAsianWidth.txt",
        "a0cf29eacd00cfcaec4381c6b7c281685f18dbb4e7ff82b4076ccb342ca839aa",
    ),
    (
        "extracted/DerivedGeneralCategory.txt",
        "d6b151d2d40ee9b1876d26f417980f45ffae47b6055ccf7203cb31f07a030f94",
    ),
    (
        "DerivedCoreProperties.txt",
        "09c928886a178fcafd93c29e4bd59073a058e5a100b716d425cb563ab50f68c9",
    ),
    (
        "auxiliary/GraphemeBreakProperty.txt",
        "0839dcb79e4ac639ecd538b1abf7c9d22e3f9dd265b7e182d33627aa4d75b45a",
    ),
    (
        "auxiliary/GraphemeBreakTest.txt",
        "b0cf047ee94485bbdc846de2b902f5f8a815f6b674f9d04223cddadd91c9df31",
    ),
    (
        "emoji/emoji-data.txt",
        "80d00f8e616a0ef27fd6b8de3b758c06383b5d917e2977709578e68baf733bf1",
    ),
    (
        "emoji/emoji-variation-sequences.txt",
        "ff1707564aa1f1b2fcf4ec92d609d4cb26940bc0d4dcf07f5328ad6879e84da3",
    ),
];
/// The largest pinned file is about 1.2 MB; anything far bigger is not it.
const MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
const TABLES_PATH: &str = "crates/scull-unicode/src/tables.rs";
const BREAK_TEST_PATH: &str = "crates/scull-unicode/tests/data/GraphemeBreakTest.txt";
const SCALAR_END: usize = 0x11_0000;
/// Code points per leaf block and leaf blocks per middle block, as bit counts;
/// 4/5 gave the smallest total (15.5 KB for Unicode 18) of every split from
/// 4/3 to 8/8 that divides the code space evenly.
const LOW_BITS: usize = 4;
const MID_BITS: usize = 5;
const NUMBERS_PER_LINE: usize = 24;

/// `WidthClass` variants in `scull-unicode/src/props.rs`.
const WIDTH_NAMES: [&str; 4] = ["Zero", "Narrow", "Wide", "Ambiguous"];
const ZERO: u8 = 0;
const NARROW: u8 = 1;
const WIDE: u8 = 2;
const AMBIGUOUS: u8 = 3;
/// UCD `Grapheme_Cluster_Break` value and the matching `Gcb` variant; index 0 is the default.
const GCB_NAMES: [(&str, &str); 14] = [
    ("Other", "Other"),
    ("CR", "Cr"),
    ("LF", "Lf"),
    ("Control", "Control"),
    ("Extend", "Extend"),
    ("ZWJ", "Zwj"),
    ("Regional_Indicator", "RegionalIndicator"),
    ("Prepend", "Prepend"),
    ("SpacingMark", "SpacingMark"),
    ("L", "L"),
    ("V", "V"),
    ("T", "T"),
    ("LV", "Lv"),
    ("LVT", "Lvt"),
];
/// UCD `InCB` value and the `InCb` variant name; index 0 is the default.
const INCB_NAMES: [&str; 4] = ["None", "Linker", "Consonant", "Extend"];
/// General categories that take no cell: marks drawn over their base,
/// invisible format characters, and controls the parser executes.
const ZERO_WIDTH_CATEGORIES: [&str; 4] = ["Mn", "Me", "Cf", "Cc"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Props {
    width: u8,
    gcb: u8,
    incb: u8,
    ext_pict: bool,
    vs_base: bool,
}

fn main() -> Result<()> {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(other) => bail!("unknown argument {other:?}; usage: scull-ucd-gen [--check]"),
    };
    let root = workspace_root()?;
    let cache = cache_dir(&root);
    fs::create_dir_all(&cache).with_context(|| format!("creating {}", cache.display()))?;
    for (path, _) in FILES {
        let dest = cache.join(file_name(path));
        if !dest.exists() {
            let url = format!("{}/{path}", base_url());
            fs::write(&dest, download(&url)?)
                .with_context(|| format!("writing {}", dest.display()))?;
        }
    }
    let mut stale = Vec::new();
    for (rel, content) in render(&cache)? {
        let dest = root.join(rel);
        if check {
            if fs::read(&dest).ok().as_deref() != Some(content.as_slice()) {
                stale.push(rel);
            }
        } else {
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            }
            fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;
        }
    }
    ensure!(
        stale.is_empty(),
        "stale generated files {stale:?}; run `just ucd`"
    );
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent);
    root.map(Path::to_path_buf)
        .context("generator is not two levels below the workspace root")
}

fn cache_dir(root: &Path) -> PathBuf {
    let (major, minor, update) = VERSION;
    root.join(format!("target/ucd/{major}.{minor}.{update}"))
}

fn base_url() -> String {
    let (major, minor, update) = VERSION;
    format!("https://www.unicode.org/Public/{major}.{minor}.{update}/ucd")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn download(url: &str) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let body = response.body_mut().with_config().limit(MAX_DOWNLOAD_BYTES);
    body.read_to_vec().with_context(|| format!("reading {url}"))
}

/// Reads a cached file and refuses it unless it is the pinned release.
fn load(cache: &Path, path: &str, sha256: &str) -> Result<String> {
    let file = cache.join(file_name(path));
    let bytes = fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    ensure!(
        actual == sha256,
        "{} has SHA-256 {actual}, expected {sha256}",
        file.display()
    );
    String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", file.display()))
}

/// `(first, last, fields)` for every data line of a UCD file. A sequence such
/// as `0023 FE0F` in the variation-sequence file yields its first code point.
fn records(text: &str) -> Result<Vec<(usize, usize, Vec<&str>)>> {
    let data = text
        .lines()
        .map(|l| l.split_once('#').map_or(l, |(d, _)| d).trim());
    data.filter(|l| !l.is_empty())
        .map(|line| {
            let mut fields = line.split(';').map(str::trim);
            let first = fields.next().and_then(|f| f.split_whitespace().next());
            let range = first.with_context(|| format!("no code point in {line:?}"))?;
            let (lo, hi) = range.split_once("..").unwrap_or((range, range));
            let hex = |s: &str| {
                usize::from_str_radix(s, 16).with_context(|| format!("bad code point in {line:?}"))
            };
            let (lo, hi) = (hex(lo)?, hex(hi)?);
            ensure!(lo <= hi && hi < SCALAR_END, "bad range in {line:?}");
            Ok((lo, hi, fields.collect()))
        })
        .collect()
}

fn index_of(names: &[&str], name: &str) -> Result<u8> {
    let i = names
        .iter()
        .position(|n| *n == name)
        .with_context(|| format!("unknown value {name:?}"))?;
    u8::try_from(i).context("too many property values")
}

fn build_props(cache: &Path) -> Result<Vec<Props>> {
    let [eaw, gc, core, gcb, _, emoji, vs] = FILES.map(|(path, sha)| load(cache, path, sha));
    let mut props = vec![
        Props {
            width: NARROW,
            gcb: 0,
            incb: 0,
            ext_pict: false,
            vs_base: false
        };
        SCALAR_END
    ];
    let ucd_gcb: Vec<&str> = GCB_NAMES.iter().map(|(ucd, _)| *ucd).collect();
    for (lo, hi, f) in records(&eaw?)? {
        let width = match f.first().copied() {
            Some("W" | "F") => WIDE,
            Some("A") => AMBIGUOUS,
            _ => NARROW,
        };
        props[lo..=hi].iter_mut().for_each(|p| p.width = width);
    }
    for (lo, hi, f) in records(&gcb?)? {
        let value = index_of(&ucd_gcb, f.first().copied().unwrap_or_default())?;
        props[lo..=hi].iter_mut().for_each(|p| p.gcb = value);
    }
    for (lo, hi, f) in records(&core?)? {
        if let ["InCB", value] = f.as_slice() {
            let value = index_of(&INCB_NAMES, value)?;
            props[lo..=hi].iter_mut().for_each(|p| p.incb = value);
        }
    }
    for (lo, hi, f) in records(&emoji?)? {
        if f.first() == Some(&"Extended_Pictographic") {
            props[lo..=hi].iter_mut().for_each(|p| p.ext_pict = true);
        }
    }
    for (lo, _, _) in records(&vs?)? {
        props[lo].vs_base = true;
    }
    for (lo, hi, f) in records(&gc?)? {
        if f.first().is_some_and(|c| ZERO_WIDTH_CATEGORIES.contains(c)) {
            props[lo..=hi].iter_mut().for_each(|p| p.width = ZERO);
        }
    }
    let [v, t, ri] = ["V", "T", "Regional_Indicator"].map(|n| index_of(&ucd_gcb, n));
    let (v, t, ri) = (v?, t?, ri?);
    for p in &mut props {
        // Medial vowels and final consonants draw inside the leading consonant's cells.
        if p.gcb == v || p.gcb == t {
            p.width = ZERO;
        }
        // Fonts draw a lone regional indicator as a wide letter, and a flag
        // must not grow when its second half arrives.
        if p.gcb == ri {
            p.width = WIDE;
        }
    }
    Ok(props)
}

/// Splits `items` into blocks of `block` entries and stores each distinct
/// block once; returns the stored blocks and, per block, its index.
fn dedupe<T: Ord + Clone>(items: &[T], block: usize) -> (Vec<T>, Vec<usize>) {
    let mut seen: BTreeMap<&[T], usize> = BTreeMap::new();
    let mut stored = Vec::new();
    let ids = items
        .chunks(block)
        .map(|chunk| {
            let next = seen.len();
            let id = *seen.entry(chunk).or_insert(next);
            if id == next {
                stored.extend_from_slice(chunk);
            }
            id
        })
        .collect();
    (stored, ids)
}

fn emit_array(out: &mut String, name: &str, values: &[usize]) -> Result<()> {
    let ty = if values.iter().all(|&v| v <= usize::from(u8::MAX)) {
        "u8"
    } else {
        "u16"
    };
    ensure!(
        values.iter().all(|&v| v <= usize::from(u16::MAX)),
        "{name} does not fit u16"
    );
    writeln!(
        out,
        "pub(crate) static {name}: [{ty}; {}] = [",
        values.len()
    )?;
    for line in values.chunks(NUMBERS_PER_LINE) {
        let line: Vec<String> = line.iter().map(usize::to_string).collect();
        writeln!(out, "    {},", line.join(", "))?;
    }
    writeln!(out, "];")?;
    Ok(())
}

fn render_tables(props: &[Props]) -> Result<String> {
    let mut distinct: BTreeMap<Props, usize> = BTreeMap::new();
    for p in props {
        let next = distinct.len();
        distinct.entry(*p).or_insert(next);
    }
    let per_cp: Vec<usize> = props
        .iter()
        .map(|p| distinct.get(p).copied().unwrap_or_default())
        .collect();
    let (leaf, leaf_ids) = dedupe(&per_cp, 1 << LOW_BITS);
    let (mid, top) = dedupe(&leaf_ids, 1 << MID_BITS);
    let (major, minor, update) = VERSION;
    let mut out = format!(
        "//! @generated by `tools/scull-ucd-gen` from the Unicode {major}.{minor}.{update} data files; do not edit.\n\
         //! Three-stage property lookup for `props.rs`, kept in its own module so\n\
         //! regeneration never touches hand-written code. Run `just ucd` to\n\
         //! regenerate. Derived from the Unicode Character Database,\n\
         //! (c) Unicode, Inc., under the Unicode License v3 (https://www.unicode.org/license.txt).\n\n\
         use crate::props::{{Gcb, InCb, Props, WidthClass}};\n\n\
         pub(crate) const UNICODE_VERSION: (u8, u8, u8) = ({major}, {minor}, {update});\n\
         pub(crate) const LOW_BITS: usize = {LOW_BITS};\n\
         pub(crate) const MID_BITS: usize = {MID_BITS};\n\n"
    );
    let mut ordered: Vec<(&Props, &usize)> = distinct.iter().collect();
    ordered.sort_by_key(|(_, i)| **i);
    writeln!(
        out,
        "pub(crate) static PROPS: [Props; {}] = [",
        ordered.len()
    )?;
    for (p, _) in ordered {
        let (_, gcb) = GCB_NAMES[usize::from(p.gcb)];
        writeln!(
            out,
            "    Props {{ width: WidthClass::{}, gcb: Gcb::{gcb}, incb: InCb::{}, ext_pict: {}, vs_base: {} }},",
            WIDTH_NAMES[usize::from(p.width)],
            INCB_NAMES[usize::from(p.incb)],
            p.ext_pict,
            p.vs_base
        )?;
    }
    writeln!(out, "];")?;
    emit_array(&mut out, "TOP", &top)?;
    emit_array(&mut out, "MID", &mid)?;
    emit_array(&mut out, "LEAF", &leaf)?;
    Ok(out)
}

/// Every committed output, keyed by its path below the workspace root.
fn render(cache: &Path) -> Result<Vec<(&'static str, Vec<u8>)>> {
    let tables = render_tables(&build_props(cache)?)?;
    let fixture = FILES
        .iter()
        .find(|(path, _)| path.ends_with("GraphemeBreakTest.txt"));
    let (path, sha) = fixture.context("GraphemeBreakTest.txt is not pinned")?;
    let break_test = load(cache, path, sha)?;
    Ok(vec![
        (TABLES_PATH, tables.into_bytes()),
        (BREAK_TEST_PATH, break_test.into_bytes()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_outputs_match_the_pinned_data_files() {
        let root = workspace_root().unwrap();
        let cache = cache_dir(&root);
        if FILES
            .iter()
            .any(|(path, _)| !cache.join(file_name(path)).exists())
        {
            eprintln!(
                "skipped: no Unicode data cache at {}; `just ucd-check` fills it",
                cache.display()
            );
            return;
        }
        for (rel, want) in render(&cache).unwrap() {
            assert!(
                fs::read(root.join(rel)).unwrap() == want,
                "{rel} is stale; run `just ucd`"
            );
        }
    }

    #[test]
    fn records_read_ranges_single_points_and_sequences() {
        let text = "# header\n0000..001F ; Cc # comment\n00AD ; Cf\n0023 FE0F ; emoji style; # x\n";
        let got = records(text).unwrap();
        assert_eq!(got[0], (0, 0x1F, vec!["Cc"]));
        assert_eq!(got[1], (0xAD, 0xAD, vec!["Cf"]));
        assert_eq!(got[2], (0x23, 0x23, vec!["emoji style", ""]));
    }

    #[test]
    fn dedupe_stores_each_distinct_block_once() {
        let (stored, ids) = dedupe(&[1, 2, 1, 2, 3, 4], 2);
        assert_eq!(stored, [1, 2, 3, 4]);
        assert_eq!(ids, [0, 0, 1]);
    }
}
