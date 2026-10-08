//! Conformance runner: replays every case in `tests/fixtures/*.txt` against
//! a fresh terminal and compares screen text, cursor, styles and replies.
//! The cases and their esctest2 and vttest sources are listed in
//! `tests/README.md`, which also describes the fixture format.
//!
//! A parallel runner rather than `scull-harness`: that crate's fixture is a
//! bare frame for a stub with no cursor, styles or replies.

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::Path;

    use scull_grid::{Attrs, CellFlags, Color, Content, Row, Style, Underline};
    use scull_term::{Event, Terminal, TitleWhich};

    /// One case: a byte stream and what it must leave behind.
    #[derive(Debug, Default)]
    struct Case {
        name: String,
        file: String,
        cols: u16,
        rows: u16,
        scrollback: usize,
        input: Vec<u8>,
        cursor: Option<String>,
        screen: Vec<String>,
        styles: Vec<(u16, u16, String)>,
        history: Option<usize>,
        replies: Option<Vec<u8>>,
        events: Option<Vec<String>>,
        links: Option<Vec<String>>,
    }

    fn unescape(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                continue;
            }
            match chars.next() {
                Some('e') => out.push(0x1b),
                Some('r') => out.push(b'\r'),
                Some('n') => out.push(b'\n'),
                Some('t') => out.push(b'\t'),
                Some('b') => out.push(0x08),
                Some('\\') => out.push(b'\\'),
                Some('s') => out.push(b' '),
                Some('x') => {
                    let hex: String = chars.by_ref().take(2).collect();
                    out.push(u8::from_str_radix(&hex, 16).expect("\\x needs two hex digits"));
                }
                other => panic!("unknown escape \\{other:?}"),
            }
        }
        out
    }

    fn parse(file: &str, text: &str) -> Vec<Case> {
        let mut cases = Vec::new();
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix("=== ") {
                cases.push(Case {
                    name: name.to_owned(),
                    file: file.to_owned(),
                    cols: 10,
                    rows: 5,
                    ..Case::default()
                });
                continue;
            }
            let case = cases.last_mut().expect("a field before the first ===");
            let (key, value) = line.split_once(':').expect("`key: value`");
            let value = value.trim();
            match key {
                "source" => {}
                "size" => {
                    let (c, r) = value.split_once('x').expect("size: COLSxROWS");
                    case.cols = c.parse().unwrap();
                    case.rows = r.parse().unwrap();
                }
                "scrollback" => case.scrollback = value.parse().unwrap(),
                "input" => case.input.extend(unescape(value)),
                "cursor" => case.cursor = Some(value.to_owned()),
                "history" => case.history = Some(value.parse().unwrap()),
                "replies" => case.replies = Some(unescape(value)),
                "style" => {
                    let (at, style) = value.split_once(' ').unwrap_or((value, ""));
                    let (r, c) = at.split_once(',').expect("style: ROW,COL attrs");
                    case.styles.push((
                        r.parse().unwrap(),
                        c.parse().unwrap(),
                        style.trim().to_owned(),
                    ));
                }
                "screen" => case.screen = pipes(&mut lines),
                "events" => case.events = Some(pipes(&mut lines)),
                "links" => case.links = Some(pipes(&mut lines)),
                other => panic!("{file}: unknown field {other}"),
            }
        }
        cases
    }

    fn pipes(lines: &mut std::iter::Peekable<std::str::Lines<'_>>) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(row) = lines.peek().and_then(|line| line.trim().strip_prefix('|')) {
            out.push(row.strip_suffix('|').expect("row ends in |").to_owned());
            lines.next();
        }
        out
    }

    fn show_event(event: &Event) -> String {
        match event {
            Event::Bell => "bell".to_owned(),
            Event::Title { which, text } => {
                let code: u8 = match which {
                    TitleWhich::Both => 0,
                    TitleWhich::Icon => 1,
                    TitleWhich::Window => 2,
                };
                format!("title {code} {text}")
            }
            Event::WorkingDirectory(uri) => format!("cwd {uri}"),
            Event::Shell { mark, extra } => {
                if extra.is_empty() {
                    format!("shell {mark}")
                } else {
                    format!("shell {mark} {extra}")
                }
            }
            Event::Clipboard(clip) => {
                let op = if clip.read { "read" } else { "write" };
                let sel = char::from(clip.selection);
                if clip.read {
                    format!("clip {op} {sel}")
                } else {
                    format!("clip {op} {sel} {}", String::from_utf8_lossy(&clip.data))
                }
            }
            Event::Link { id, uri } => format!("link {id} {uri}"),
            Event::Notification { title, body } => format!("notify {title}|{body}"),
        }
    }

    fn link_dump(term: &Terminal, rows: u16) -> Vec<String> {
        let mut out = Vec::new();
        for row in 0..rows {
            let Some(line) = term.grid().screen_row(row) else {
                continue;
            };
            for span in line.links() {
                out.push(format!(
                    "{} {}..{}={}",
                    row + 1,
                    span.start,
                    span.end,
                    span.link.0
                ));
            }
        }
        out
    }

    fn row_text(term: &Terminal, row: &Row) -> String {
        let mut out = String::new();
        for cell in row.cells() {
            if cell.flags().contains(CellFlags::SPACER) {
                continue;
            }
            match cell.content() {
                Content::Empty => out.push('.'),
                Content::Char(c) => out.push(c),
                Content::Cluster(id) => out.push_str(term.grid().clusters().get(id).unwrap_or("?")),
            }
        }
        out
    }

    fn color(c: Color) -> String {
        match c {
            Color::Default => "default".to_owned(),
            Color::Indexed(n) => n.to_string(),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }

    fn style_text(style: &Style) -> String {
        let names = [
            (Attrs::BOLD, "bold"),
            (Attrs::DIM, "dim"),
            (Attrs::ITALIC, "italic"),
            (Attrs::BLINK, "blink"),
            (Attrs::INVERSE, "inverse"),
            (Attrs::HIDDEN, "hidden"),
            (Attrs::STRIKE, "strike"),
            (Attrs::OVERLINE, "overline"),
        ];
        let mut parts: Vec<String> = names
            .iter()
            .filter(|(a, _)| style.attrs.contains(*a))
            .map(|(_, n)| (*n).to_owned())
            .collect();
        if style.fg != Color::Default {
            parts.push(format!("fg={}", color(style.fg)));
        }
        if style.bg != Color::Default {
            parts.push(format!("bg={}", color(style.bg)));
        }
        if style.underline != Underline::None {
            parts.push(format!("ul={:?}", style.underline).to_lowercase());
        }
        if style.underline_color != Color::Default {
            parts.push(format!("ulc={}", color(style.underline_color)));
        }
        if parts.is_empty() {
            "default".to_owned()
        } else {
            parts.join(" ")
        }
    }

    /// Runs one case; returns a description of every mismatch.
    fn check(case: &Case) -> String {
        let mut term = Terminal::new(case.cols, case.rows, case.scrollback).unwrap();
        term.feed(&case.input);
        let mut errors = String::new();
        if let Some(want) = &case.replies {
            let got = term.take_replies();
            if &got != want {
                let _ = writeln!(
                    errors,
                    "  replies: want {:?}, got {:?}",
                    String::from_utf8_lossy(want),
                    String::from_utf8_lossy(&got)
                );
            }
        }
        let grid = term.grid();
        if !case.screen.is_empty() {
            let got: Vec<String> = (0..case.rows)
                .map(|r| row_text(&term, grid.screen_row(r).unwrap()))
                .collect();
            if got != case.screen {
                let _ = writeln!(
                    errors,
                    "  screen:\n    want {:?}\n    got  {:?}",
                    case.screen, got
                );
            }
        }
        if let Some(want) = &case.cursor {
            let c = term.cursor();
            let mut got = format!("{},{}", c.row + 1, c.col + 1);
            if c.pending_wrap {
                got.push_str(" wrap");
            }
            if &got != want {
                let _ = writeln!(errors, "  cursor: want {want}, got {got}");
            }
        }
        for (r, c, want) in &case.styles {
            let cell = grid
                .screen_row(r - 1)
                .and_then(|row| row.cell(c - 1))
                .unwrap();
            let got = style_text(grid.styles().get(cell.style()).unwrap());
            if &got != want {
                let _ = writeln!(errors, "  style {r},{c}: want {want:?}, got {got:?}");
            }
        }
        if let Some(want) = case.history
            && grid.history_len() != want
        {
            let _ = writeln!(errors, "  history: want {want}, got {}", grid.history_len());
        }
        if let Some(want) = &case.events {
            let mut got = Vec::new();
            while let Some(event) = term.poll_event() {
                got.push(show_event(&event));
            }
            if &got != want {
                let _ = writeln!(errors, "  events:\n    want {want:?}\n    got  {got:?}");
            }
        }
        if let Some(want) = &case.links {
            let got = link_dump(&term, case.rows);
            if &got != want {
                let _ = writeln!(errors, "  links:\n    want {want:?}\n    got  {got:?}");
            }
        }
        errors
    }

    #[test]
    fn every_fixture_case_matches_its_expected_state() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        let mut total = 0;
        let mut failures = String::new();
        for path in files {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            for case in parse(&file, &fs::read_to_string(&path).unwrap()) {
                total += 1;
                assert!(
                    case.cursor.is_some()
                        || !case.screen.is_empty()
                        || !case.styles.is_empty()
                        || case.history.is_some()
                        || case.replies.is_some()
                        || case.events.is_some()
                        || case.links.is_some(),
                    "{}: {} checks nothing",
                    case.file,
                    case.name
                );
                let errors = check(&case);
                if !errors.is_empty() {
                    let _ = writeln!(failures, "{}: {}\n{errors}", case.file, case.name);
                }
            }
        }
        assert!(total > 0, "no fixture cases found");
        assert!(failures.is_empty(), "failing cases:\n{failures}");
    }
}
