//! Parser throughput against `vte`, the parser Alacritty uses, on the same
//! inputs: plain ASCII lines, UTF-8 lines, and colourful SGR-heavy output.
//! Separate from the T3 harness because it measures the parser alone, with
//! handlers that only count, so the number is the parser's own cost.
//!
//! Run: `mise exec -- cargo bench -p scull-parser --bench throughput`.

use std::hint::black_box;

use divan::Bencher;
use divan::counter::BytesCount;

fn main() {
    divan::main();
}

/// Size of every generated input; large enough that one sample is many
/// PTY reads, small enough for a quick run.
const INPUT_BYTES: usize = 1 << 20;
/// A typical terminal width.
const LINE_CHARS: usize = 80;
/// The 256-colour palette size, for cycling SGR colours.
const PALETTE: usize = 256;
/// Large primes keep the generated truecolour values from repeating early.
const RED_STEP: usize = 37;
const GREEN_STEP: usize = 91;
const BLUE_STEP: usize = 53;

const ASCII_WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
];
const UTF8_WORDS: &[&str] = &[
    "привет",
    "мир",
    "日本語",
    "テキスト",
    "🎉",
    "émoji",
    "Straße",
    "αβγ",
    "한국어",
    "✓",
];

fn lines(words: &[&str]) -> Vec<u8> {
    let mut out = String::with_capacity(INPUT_BYTES);
    let mut line_len = 0;
    for word in words.iter().cycle() {
        if out.len() >= INPUT_BYTES {
            break;
        }
        if line_len + word.chars().count() >= LINE_CHARS {
            out.push_str("\r\n");
            line_len = 0;
        }
        out.push_str(word);
        out.push(' ');
        line_len += word.chars().count() + 1;
    }
    out.into_bytes()
}

/// `ls --color` and compiler output style: most words carry their own colour.
fn sgr_heavy() -> Vec<u8> {
    let mut out = String::with_capacity(INPUT_BYTES);
    for (i, word) in ASCII_WORDS.iter().cycle().enumerate() {
        if out.len() >= INPUT_BYTES {
            break;
        }
        if i % 2 == 0 {
            out.push_str(&format!("\x1b[1;38;5;{}m{word}\x1b[0m ", i % PALETTE));
        } else {
            let (r, g, b) = (
                (i * RED_STEP) % PALETTE,
                (i * GREEN_STEP) % PALETTE,
                (i * BLUE_STEP) % PALETTE,
            );
            out.push_str(&format!("\x1b[38;2;{r};{g};{b}m{word}\x1b[m "));
        }
        if i % ASCII_WORDS.len() == ASCII_WORDS.len() - 1 {
            out.push_str("\r\n");
        }
    }
    out.into_bytes()
}

#[derive(Default)]
struct ScullCount(usize);

impl scull_parser::Handler for ScullCount {
    fn print(&mut self, text: &str) {
        self.0 += text.len();
    }
    fn csi_dispatch(&mut self, _csi: &scull_parser::Csi<'_>) {
        self.0 += 1;
    }
}

#[derive(Default)]
struct VteCount(usize);

impl vte::Perform for VteCount {
    fn print(&mut self, c: char) {
        self.0 += c.len_utf8();
    }
    fn csi_dispatch(&mut self, _: &vte::Params, _: &[u8], _: bool, _: char) {
        self.0 += 1;
    }
}

fn run_scull(bencher: Bencher, input: &[u8]) {
    bencher
        .counter(BytesCount::of_slice(input))
        .bench_local(|| {
            let mut parser = scull_parser::Parser::new();
            let mut count = ScullCount::default();
            parser.feed(black_box(input), &mut count);
            black_box(count.0)
        });
}

fn run_vte(bencher: Bencher, input: &[u8]) {
    bencher
        .counter(BytesCount::of_slice(input))
        .bench_local(|| {
            let mut parser = vte::Parser::new();
            let mut count = VteCount::default();
            parser.advance(&mut count, black_box(input));
            black_box(count.0)
        });
}

mod ascii {
    use super::*;

    #[divan::bench]
    fn scull(bencher: Bencher) {
        run_scull(bencher, &lines(ASCII_WORDS));
    }

    #[divan::bench]
    fn vte(bencher: Bencher) {
        run_vte(bencher, &lines(ASCII_WORDS));
    }
}

mod utf8 {
    use super::*;

    #[divan::bench]
    fn scull(bencher: Bencher) {
        run_scull(bencher, &lines(UTF8_WORDS));
    }

    #[divan::bench]
    fn vte(bencher: Bencher) {
        run_vte(bencher, &lines(UTF8_WORDS));
    }
}

mod sgr {
    use super::*;

    #[divan::bench]
    fn scull(bencher: Bencher) {
        run_scull(bencher, &sgr_heavy());
    }

    #[divan::bench]
    fn vte(bencher: Bencher) {
        run_vte(bencher, &sgr_heavy());
    }
}
