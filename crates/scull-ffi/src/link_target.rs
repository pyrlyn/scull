//! What a click on an OSC 8 link may open, decided once for both shells.
//! The URI is whatever the program in the terminal wrote, so only targets a
//! user expects a click to reach pass: a web page, a mail address, or a path
//! on this machine. A shell opens a web or mail target with the system, opens
//! a directory in its file manager and only reveals any other file there, so
//! a click never runs a program the terminal's program wrote.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::fs;

use percent_encoding::percent_decode_str;
use scull_term::MAX_URI;

use crate::guard::{guard, tt_status};
use crate::spawn::{text, tt_str};
use crate::text::{copy_out, out_ok};

/// `tt_link_target` kind: nothing to open.
pub const TT_LINK_REFUSED: u32 = 0;
/// An http or https page; the target is the URI.
pub const TT_LINK_WEB: u32 = 1;
/// A mailto address; the target is the URI.
pub const TT_LINK_MAIL: u32 = 2;
/// A directory on this machine, to open in the file manager; the target is
/// its path.
pub const TT_LINK_DIRECTORY: u32 = 3;
/// Any other existing file on this machine, to reveal in the file manager
/// and never open; the target is its path.
pub const TT_LINK_FILE: u32 = 4;

/// A target before the file system is asked whether a path is a directory.
#[derive(Debug, PartialEq, Eq)]
enum Target<'a> {
    Web(&'a str),
    Mail(&'a str),
    Local(String),
}

/// Sorts `uri`. `machine` is this computer's name, which a file URL may
/// give as its host; `windows` picks the path style.
fn classify<'a>(uri: &'a str, machine: &str, windows: bool) -> Option<Target<'a>> {
    // Whitespace, controls and bidi overrides never belong in a URI and are
    // how a link shows the user one thing and opens another.
    if uri.len() > MAX_URI
        || uri
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || is_bidi(c))
    {
        return None;
    }
    let (scheme, rest) = uri.split_once(':')?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => web(rest).then_some(Target::Web(uri)),
        "mailto" => (!rest.is_empty()).then_some(Target::Mail(uri)),
        "file" => local(rest, machine, windows).map(Target::Local),
        _ => None,
    }
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn web(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix("//") else {
        return false;
    };
    let authority = after.split(['/', '?', '#']).next().unwrap_or_default();
    // Userinfo lets a link read as one host and go to another
    // (`https://bank.example@evil.example`).
    let host = authority.split(':').next().unwrap_or_default();
    !authority.contains('@') && !host.is_empty()
}

fn local(rest: &str, machine: &str, windows: bool) -> Option<String> {
    let after = rest.strip_prefix("//")?;
    let (host, path) = after.split_at(after.find('/')?);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost") || names(host, machine)) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = percent_decode_str(path).decode_utf8().ok()?;
    // A second leading slash or any backslash makes a share path
    // (`\\host\share`) on Windows: one click would reach another machine
    // and hand it the user's credentials.
    if path.starts_with("//") || path.contains('\\') || path.chars().any(char::is_control) {
        return None;
    }
    if !windows {
        return Some(path.into_owned());
    }
    // `/C:/Users` or `/C:`; a path without a drive would be relative to
    // whichever drive is current.
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes.len() == 3 || bytes[3] == b'/');
    if !drive {
        return None;
    }
    let mut out = path[1..].replace('/', "\\");
    if out.len() == 2 {
        out.push('\\');
    }
    Some(out)
}

/// Whether a file URL's `host` is this machine, by its full name or its
/// first label (shells write either).
fn names(host: &str, machine: &str) -> bool {
    let short = machine.split('.').next().unwrap_or_default();
    !machine.is_empty() && (host.eq_ignore_ascii_case(machine) || host.eq_ignore_ascii_case(short))
}

/// Sorts the OSC 8 `uri` a click landed on: writes a `TT_LINK_*` to
/// `*kind` and copies the target (the URI, or a local path in this
/// platform's style) into `buf` as `tt_term_link_uri` does. `machine` is
/// this computer's name, which a file URL may give as its host. A refused
/// link is `TT_OK` with `TT_LINK_REFUSED` and an empty target; so is a file
/// URL whose path does not exist. `TT_INVALID` for text that is not UTF-8
/// or a `NULL` `kind` or `len`.
///
/// # Safety
///
/// `uri` and `machine` are valid as `tt_str`; `kind` is `NULL` or
/// writable; `buf`, `cap` and `len` as for `tt_term_link_uri`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_link_target(
    uri: tt_str,
    machine: tt_str,
    kind: *mut u32,
    buf: *mut u8,
    cap: usize,
    len: *mut usize,
) -> tt_status {
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(uri), Some(machine)) = (unsafe { text(uri) }, unsafe { text(machine) }) else {
            return tt_status::TT_INVALID;
        };
        if kind.is_null() || !out_ok(buf, cap, len) {
            return tt_status::TT_INVALID;
        }
        let (found, target) = match classify(uri, machine, cfg!(windows)) {
            Some(Target::Web(uri)) => (TT_LINK_WEB, uri.to_owned()),
            Some(Target::Mail(uri)) => (TT_LINK_MAIL, uri.to_owned()),
            Some(Target::Local(path)) => match fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => (TT_LINK_DIRECTORY, path),
                Ok(_) => (TT_LINK_FILE, path),
                Err(_) => (TT_LINK_REFUSED, String::new()),
            },
            None => (TT_LINK_REFUSED, String::new()),
        };
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { kind.write(found) };
        // SAFETY: the caller's contract, checked by `out_ok`.
        unsafe { copy_out(target.as_bytes(), buf, cap, len) }
    })
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    fn sort(uri: &str) -> Option<Target<'_>> {
        classify(uri, "MyMac.local", false)
    }

    fn on_windows(uri: &str) -> Option<String> {
        match classify(uri, "MYPC", true)? {
            Target::Local(path) => Some(path),
            _ => None,
        }
    }

    #[test]
    fn web_and_mail_links_pass_in_any_scheme_case() {
        for uri in [
            "https://example.com/a?b=1#c",
            "HTTP://example.com",
            "https://[::1]:8080/x",
        ] {
            assert_eq!(sort(uri), Some(Target::Web(uri)), "{uri}");
        }
        assert_eq!(
            sort("MailTo:a@example.com"),
            Some(Target::Mail("MailTo:a@example.com"))
        );
    }

    #[test]
    fn other_schemes_and_hostless_web_links_are_refused() {
        for uri in [
            "javascript:alert(1)",
            "ftp://example.com",
            "data:text/html,x",
            "x-apple.systempreferences:",
            "ms-settings:",
            "search-ms:query=x",
            "https://",
            "https:example.com",
            "https://:80/x",
            "mailto:",
            "/etc/passwd",
            "C:\\Windows",
            "not a url",
            "",
        ] {
            assert_eq!(sort(uri), None, "{uri}");
        }
    }

    #[test]
    fn userinfo_whitespace_controls_and_long_uris_are_refused() {
        for uri in [
            "https://a@b.example",
            "https://bank.example:x@evil.example/",
            "https://example.com/a b",
            "https://example.com/\u{a0}",
            "https://example.com/\u{7}",
            "https://example.com/\u{202e}",
            "file:///tmp/a\tb",
        ] {
            assert_eq!(sort(uri), None, "{uri:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URI));
        assert_eq!(sort(&long), None);
        let fits = format!("https://example.com/{}", "a".repeat(MAX_URI - 20));
        assert!(sort(&fits).is_some());
    }

    #[test]
    fn a_file_url_naming_this_machine_is_its_decoded_path() {
        let local = |uri| match sort(uri) {
            Some(Target::Local(path)) => Some(path),
            _ => None,
        };
        assert_eq!(local("file:///tmp/a%20b").as_deref(), Some("/tmp/a b"));
        assert_eq!(local("file://localhost/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(local("file://MYMAC/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(
            local("file://mymac.local/tmp/x?q#f").as_deref(),
            Some("/tmp/x")
        );
        assert_eq!(local("file://other/tmp/x"), None, "another machine's path");
        assert_eq!(local("file://mymac.evil/tmp/x"), None);
        assert_eq!(local("file:/tmp/x"), None);
        assert_eq!(local("file:///tmp/%00x"), None);
        assert_eq!(local("file:///tmp/%FFx"), None, "not UTF-8");
    }

    #[test]
    fn a_path_that_could_name_a_share_is_refused() {
        for uri in [
            "file:////h/s/x",
            "file://localhost//h/s/x",
            "file:///\\\\h\\s\\x",
            "file:///%5C%5Ch/s",
            "file:///%2F%2Fh/s",
            "file:///C:/a\\..\\b",
        ] {
            assert_eq!(sort(uri), None, "{uri}");
            assert_eq!(on_windows(uri), None, "{uri}");
        }
    }

    #[test]
    fn windows_paths_need_a_drive() {
        assert_eq!(
            on_windows("file:///C:/Users/me/a%20b").as_deref(),
            Some("C:\\Users\\me\\a b")
        );
        assert_eq!(on_windows("file://mypc/d:/x").as_deref(), Some("d:\\x"));
        assert_eq!(on_windows("file:///C:").as_deref(), Some("C:\\"));
        assert_eq!(on_windows("file:///home/me"), None);
        assert_eq!(on_windows("file:///C:x"), None);
    }

    fn target(uri: &str) -> (tt_status, u32, String) {
        let mut kind = u32::MAX;
        let mut buf = [0u8; 256];
        let mut len = 0;
        let as_str = |s: &str| tt_str {
            ptr: s.as_ptr(),
            len: s.len(),
        };
        // SAFETY: valid strings, a whole buffer and writable outputs.
        let status = unsafe {
            tt_link_target(
                as_str(uri),
                as_str(""),
                &mut kind,
                buf.as_mut_ptr(),
                buf.len(),
                &mut len,
            )
        };
        let text = String::from_utf8(buf[..len.min(buf.len())].to_vec()).unwrap();
        (status, kind, text)
    }

    #[test]
    fn the_export_tells_a_directory_from_a_file_and_drops_a_missing_path() {
        let dir = std::env::temp_dir().join(format!("scull-link-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a b.txt");
        fs::write(&file, b"x").unwrap();
        let url = |p: &std::path::Path| {
            let p = p.to_str().unwrap().replace('\\', "/").replace(' ', "%20");
            if p.starts_with('/') {
                format!("file://{p}")
            } else {
                format!("file:///{p}")
            }
        };
        let (status, kind, path) = target(&url(&dir));
        assert_eq!((status, kind), (tt_status::TT_OK, TT_LINK_DIRECTORY));
        assert_eq!(
            fs::canonicalize(path).unwrap(),
            fs::canonicalize(&dir).unwrap()
        );
        assert_eq!(target(&url(&file)).1, TT_LINK_FILE);
        assert_eq!(
            target(&url(&dir.join("missing"))),
            (tt_status::TT_OK, TT_LINK_REFUSED, String::new())
        );
        assert_eq!(
            target("https://example.com"),
            (tt_status::TT_OK, TT_LINK_WEB, "https://example.com".into())
        );
        assert_eq!(
            target("javascript:x"),
            (tt_status::TT_OK, TT_LINK_REFUSED, String::new())
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_text_and_null_outputs_are_refused() {
        let bad = [0xFFu8];
        let mut kind = 0;
        let mut len = 0;
        let empty = tt_str {
            ptr: ptr::null(),
            len: 0,
        };
        // SAFETY: readable strings and writable or NULL outputs.
        unsafe {
            let not_utf8 = tt_str {
                ptr: bad.as_ptr(),
                len: 1,
            };
            assert_eq!(
                tt_link_target(not_utf8, empty, &mut kind, ptr::null_mut(), 0, &mut len),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_link_target(empty, empty, ptr::null_mut(), ptr::null_mut(), 0, &mut len),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_link_target(empty, empty, &mut kind, ptr::null_mut(), 0, ptr::null_mut()),
                tt_status::TT_INVALID
            );
        }
    }
}
