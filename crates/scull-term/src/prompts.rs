//! Where the shell's prompts start. A shell that speaks OSC 133 marks each
//! prompt with `A`; the core keeps the absolute line of every one (the
//! numbering `images.rs` describes, so scrolling moves nothing and only
//! eviction, ED 2 and reflow touch them) and finds the previous or next
//! one for a host that wants to jump between them.
//!
//! Only the primary screen is marked: the alternate screen has no history
//! to jump in, and a full-screen program is not a shell prompt.

use crate::state::State;

impl State {
    /// OSC 133 `A`: a prompt starts on the cursor's line.
    pub(crate) fn record_prompt(&mut self) {
        if self.alt_active {
            return;
        }
        // The ring is bounded, so pruning here keeps the list bounded too.
        self.images.prune();
        let line = self.line_of(self.cursor.row);
        let prompts = &mut self.images.prompts;
        let at = prompts.partition_point(|&l| l < line);
        if prompts.get(at) != Some(&line) {
            prompts.insert(at, line);
        }
    }

    /// How far back into history the viewport sits when the previous
    /// (`forward` false) or next prompt is its top row, clamped to the live
    /// screen; `None` when there is no such prompt, or the next one is
    /// already in view at the bottom.
    pub(crate) fn prompt_offset(&mut self, forward: bool) -> Option<usize> {
        self.images.prune();
        let base = self.line_of(0);
        let shown = self.grid.display_offset();
        let top = base.saturating_sub(u64::try_from(shown).unwrap_or(u64::MAX));
        let prompts = &self.images.prompts;
        let line = if forward {
            prompts.iter().find(|&&l| l > top)
        } else {
            prompts.iter().rev().find(|&&l| l < top)
        }?;
        let offset = usize::try_from(base.saturating_sub(*line)).unwrap_or(usize::MAX);
        let offset = offset.min(self.grid.history_len());
        (offset != shown).then_some(offset)
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal::Terminal;

    const A: &str = "\x1b]133;A\x07";

    /// `n` numbered lines, each one a prompt, on a 3-row screen.
    fn prompts(term: &mut Terminal, n: usize) {
        for i in 0..n {
            term.feed(format!("{A}$ {i}\r\n").as_bytes());
        }
    }

    /// The line at the top of the viewport, counting from the oldest.
    fn top(term: &Terminal) -> u64 {
        term.screen_top_line() - term.grid().display_offset() as u64
    }

    #[test]
    fn previous_and_next_walk_the_prompts() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 6);
        // Prompts are on lines 0..6; the screen shows lines 4..7.
        assert_eq!(top(&term), 4);
        assert!(term.scroll_to_prompt(false));
        assert_eq!(top(&term), 3);
        assert!(term.scroll_to_prompt(false));
        assert_eq!(top(&term), 2);
        assert!(term.scroll_to_prompt(true));
        assert_eq!(top(&term), 3);
        for _ in 0..3 {
            assert!(term.scroll_to_prompt(false));
        }
        assert_eq!(top(&term), 0);
        assert!(
            !term.scroll_to_prompt(false),
            "nothing above the first prompt"
        );
        assert_eq!(top(&term), 0);
    }

    #[test]
    fn the_next_prompt_on_the_live_screen_brings_it_back() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 6);
        term.scroll_display(100);
        assert_eq!(top(&term), 0);
        assert!(term.scroll_to_prompt(true));
        assert!(term.scroll_to_prompt(true));
        assert!(term.scroll_to_prompt(true));
        assert!(term.scroll_to_prompt(true), "prompt 4 is the screen's top");
        assert_eq!(term.grid().display_offset(), 0);
        assert!(!term.scroll_to_prompt(true), "already at the bottom");
    }

    #[test]
    fn a_screen_without_marks_has_nowhere_to_jump() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        term.feed(b"a\r\nb\r\nc\r\nd\r\n");
        assert!(!term.scroll_to_prompt(false) && !term.scroll_to_prompt(true));
        term.feed(b"\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07");
        assert!(!term.scroll_to_prompt(false), "only A starts a prompt");
    }

    #[test]
    fn a_prompt_redrawn_on_its_line_is_one_mark() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        term.feed(format!("{A}$ {A}$ x\r\n{A}").as_bytes());
        assert_eq!(term.state_prompts(), vec![0, 1]);
    }

    #[test]
    fn trimmed_history_takes_its_marks_along() {
        let mut term = Terminal::new(10, 3, 2).unwrap();
        prompts(&mut term, 10);
        // The ring holds two history lines and three screen lines, so lines
        // 0..6 are gone: marks 6 to 9 stay.
        assert_eq!(term.state_prompts(), vec![6, 7, 8, 9]);
        assert!(term.scroll_to_prompt(false));
        assert_eq!(top(&term), 7);
        assert!(term.scroll_to_prompt(false));
        assert_eq!(top(&term), 6);
        assert!(!term.scroll_to_prompt(false));
    }

    #[test]
    fn clearing_the_scrollback_drops_the_marks_above_the_screen() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 6);
        term.feed(b"\x1b[3J");
        assert_eq!(term.state_prompts(), vec![4, 5]);
        assert!(!term.scroll_to_prompt(false));
    }

    #[test]
    fn clearing_the_screen_drops_the_marks_on_it() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 6);
        term.feed(b"\x1b[2J\x1b[H");
        assert_eq!(term.state_prompts(), vec![0, 1, 2, 3]);
        term.feed(format!("{A}$ ").as_bytes());
        assert_eq!(term.state_prompts(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_full_reset_keeps_the_scrollbacks_marks_and_drops_the_screens() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 6);
        term.feed(b"\x1bc");
        assert_eq!(term.state_prompts(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn the_alternate_screen_marks_nothing_and_hides_nothing() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        prompts(&mut term, 3);
        term.feed(format!("\x1b[?1049h{A}vim\x1b[?1049l").as_bytes());
        assert_eq!(term.state_prompts(), vec![0, 1, 2]);
    }

    #[test]
    fn a_rewrapping_resize_keeps_each_mark_on_its_line() {
        let mut term = Terminal::new(10, 3, 50).unwrap();
        // One prompt whose text wraps at 4 columns, then a short one.
        term.feed(format!("{A}abcdefgh\r\n{A}ij\r\n").as_bytes());
        assert_eq!(term.state_prompts(), vec![0, 1]);
        term.resize(4, 3).unwrap();
        // "abcd|efgh" now take two rows, so the second prompt moved down one.
        assert_eq!(term.state_prompts(), vec![0, 2]);
        term.resize(10, 3).unwrap();
        assert_eq!(term.state_prompts(), vec![0, 1]);
    }

    #[test]
    fn a_resize_that_evicts_a_marked_line_drops_the_mark() {
        let mut term = Terminal::new(4, 3, 1).unwrap();
        prompts(&mut term, 4);
        term.resize(2, 3).unwrap();
        // Each "$ n" now takes two rows; one history line and three screen
        // lines hold only the last prompt's rows and the cursor's, so the
        // marks of the earlier prompts went with their rows.
        assert_eq!(term.state_prompts(), vec![term.screen_top_line()]);
    }
}
