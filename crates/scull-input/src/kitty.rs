//! The kitty keyboard protocol: progressive-enhancement flags, the
//! per-screen flag stack a program drives with `CSI = > < ? u`, and the
//! `CSI … u` encoder for every flag combination. Separate from the legacy
//! encoder because once a flag is set every report changes shape.
//!
//! Implemented from the specification, not from kitty's GPL-3.0-only code:
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>.

use bitflags::bitflags;

use crate::bytes::{COLON, CSI, SEMI, push_char, push_decimal};
use crate::key::{
    CR, DEL, FINAL_TILDE, FINAL_U, Key, KeyAction, KeyEvent, Modifiers, SystemKey, TAB, functional,
};

bitflags! {
    /// Progressive-enhancement flags.
    /// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#progressive-enhancement>
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct KittyFlags: u8 {
        /// Esc, Alt+key, Ctrl+key and keypad keys become `CSI u` reports.
        const DISAMBIGUATE = 1;
        /// Repeat and release events are reported.
        const REPORT_EVENTS = 1 << 1;
        /// Shifted and base-layout keys ride along with the key code.
        const REPORT_ALTERNATES = 1 << 2;
        /// Text keys, Enter, Tab, Backspace and modifiers are reports too.
        const REPORT_ALL_KEYS = 1 << 3;
        /// Reports carry the text the key typed.
        const REPORT_TEXT = 1 << 4;
    }
}

impl KittyFlags {
    /// Flags that move a key out of legacy encoding; the other two only
    /// refine a report that is already an escape code.
    const ESCAPING: Self = Self::DISAMBIGUATE
        .union(Self::REPORT_EVENTS)
        .union(Self::REPORT_ALL_KEYS);

    /// True when key events use the legacy encoding.
    pub fn is_legacy(self) -> bool {
        !self.intersects(Self::ESCAPING)
    }

    /// Flags from a PTY parameter; undefined bits are dropped.
    pub fn from_param(raw: u16) -> Self {
        let defined = raw & u16::from(Self::all().bits());
        Self::from_bits_truncate(u8::try_from(defined).unwrap_or_default())
    }
}

/// How `CSI = flags ; mode u` combines `flags` with the current ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetMode {
    /// Mode 1, the default: replace.
    Replace,
    /// Mode 2: set the given bits.
    Union,
    /// Mode 3: clear the given bits.
    Difference,
}

impl SetMode {
    /// The mode for a PTY parameter; an omitted parameter (0) is the
    /// default, an unknown one is `None` and the request is ignored.
    pub fn from_param(raw: u16) -> Option<Self> {
        match raw {
            0 | 1 => Some(Self::Replace),
            2 => Some(Self::Union),
            3 => Some(Self::Difference),
            _ => None,
        }
    }
}

/// Entries one flag stack holds. The specification asks for a cap against
/// denial of service and for evicting the oldest entry when it is full;
/// kitty itself uses 8 (`docs/research/kitty.md` §8).
pub const KITTY_FLAG_STACK_DEPTH: usize = 8;

/// One screen's flag stack. The top entry is the active flags; an empty
/// stack means no flags. Fixed size, so a program pushing forever costs
/// nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KittyFlagStack {
    entries: [KittyFlags; KITTY_FLAG_STACK_DEPTH],
    len: usize,
}

impl KittyFlagStack {
    /// The active flags.
    pub fn current(&self) -> KittyFlags {
        self.len
            .checked_sub(1)
            .and_then(|top| self.entries.get(top))
            .copied()
            .unwrap_or_default()
    }

    /// Entries on the stack.
    pub fn depth(&self) -> usize {
        self.len
    }

    /// `CSI > flags u`: push, evicting the oldest entry when full.
    pub fn push(&mut self, flags: u16) {
        if self.len == KITTY_FLAG_STACK_DEPTH {
            self.entries.rotate_left(1);
            self.len -= 1;
        }
        if let Some(slot) = self.entries.get_mut(self.len) {
            *slot = KittyFlags::from_param(flags);
            self.len += 1;
        }
    }

    /// `CSI < count u`: pop `count` entries (0 means the default, 1).
    /// Popping past the bottom empties the stack, which resets all flags.
    pub fn pop(&mut self, count: u16) {
        let count = usize::from(count.max(1));
        self.len = self.len.saturating_sub(count);
    }

    /// `CSI = flags ; mode u`: change the active flags in place.
    pub fn set(&mut self, flags: u16, mode: u16) {
        let Some(mode) = SetMode::from_param(mode) else {
            return;
        };
        // Setting flags with nothing pushed makes them the bottom entry, so a
        // later pop still returns to "no flags".
        if self.len == 0 {
            self.push(0);
        }
        let flags = KittyFlags::from_param(flags);
        if let Some(top) = self
            .len
            .checked_sub(1)
            .and_then(|i| self.entries.get_mut(i))
        {
            *top = match mode {
                SetMode::Replace => flags,
                SetMode::Union => *top | flags,
                SetMode::Difference => *top - flags,
            };
        }
    }

    /// Empties the stack (full reset).
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The reply to `CSI ? u`: `CSI ? flags u`.
    pub fn write_query_reply(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&CSI);
        out.push(QUERY_MARKER);
        push_decimal(out, u32::from(self.current().bits()));
        out.push(FINAL_U);
    }
}

const QUERY_MARKER: u8 = b'?';

/// Which screen a stack belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The main screen.
    #[default]
    Main,
    /// The alternate screen.
    Alternate,
}

/// The two stacks the specification requires, so an editor on the
/// alternate screen cannot disturb the shell's flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KittyKeyboard {
    main: KittyFlagStack,
    alternate: KittyFlagStack,
}

impl KittyKeyboard {
    /// The stack of `screen`.
    pub fn stack(&self, screen: Screen) -> &KittyFlagStack {
        match screen {
            Screen::Main => &self.main,
            Screen::Alternate => &self.alternate,
        }
    }

    /// The stack of `screen`, mutably.
    pub fn stack_mut(&mut self, screen: Screen) -> &mut KittyFlagStack {
        match screen {
            Screen::Main => &mut self.main,
            Screen::Alternate => &mut self.alternate,
        }
    }
}

pub(crate) fn encode(event: &KeyEvent<'_>, flags: KittyFlags, out: &mut Vec<u8>) {
    let report_all = flags.contains(KittyFlags::REPORT_ALL_KEYS);
    let report_events = flags.contains(KittyFlags::REPORT_EVENTS);
    if event.action == KeyAction::Release && !report_events {
        return;
    }
    let mods = event.reported_mods();
    let plain = mods - Modifiers::LOCKS;
    let types_text = match event.key {
        Key::Char(_) => true,
        Key::Keypad(k) => k.text_char().is_some(),
        _ => false,
    };
    let release = event.action == KeyAction::Release;
    if !report_all {
        let c0 = match event.key {
            Key::Enter => Some(CR),
            Key::Tab => Some(TAB),
            Key::Backspace => Some(DEL),
            Key::Modifier(_) | Key::System(SystemKey::CapsLock | SystemKey::NumLock) => return,
            _ => None,
        };
        // Enter, Tab and Backspace stay legacy and never report a release,
        // so `reset` can still be typed after a program dies in this mode.
        if let Some(byte) = c0 {
            if release {
                return;
            }
            if plain.is_empty() {
                out.push(byte);
                return;
            }
        }
        let shift_only = plain.difference(Modifiers::SHIFT).is_empty();
        if types_text && shift_only && (release || event.has_text()) {
            if !release {
                event.clean_text().for_each(|c| push_char(out, c));
            }
            return;
        }
    }
    let (number, fin) = match event.key {
        Key::Char(base) => (u32::from(event.unshifted.unwrap_or(base)), FINAL_U),
        key => match functional(key) {
            Some(form) => form,
            None => return,
        },
    };
    // Lock states are not reported for text keys unless every key is a report.
    let mods = if types_text && !report_all {
        plain
    } else {
        mods
    };
    let event_type = (report_events && event.action != KeyAction::Press).then_some(event.action);
    let with_text = report_all
        && flags.contains(KittyFlags::REPORT_TEXT)
        && event.action != KeyAction::Release
        && event.has_text();
    let with_mods = !mods.is_empty() || event_type.is_some();

    out.extend_from_slice(&CSI);
    if fin == FINAL_U || fin == FINAL_TILDE || with_mods {
        push_decimal(out, number);
    }
    if fin == FINAL_U && flags.contains(KittyFlags::REPORT_ALTERNATES) {
        write_alternates(event, number, mods, out);
    }
    if with_mods || with_text {
        out.push(SEMI);
    }
    if with_mods {
        push_decimal(out, mods.wire());
        if let Some(action) = event_type {
            out.push(COLON);
            push_decimal(out, u32::from(action as u8));
        }
    }
    if with_text {
        out.push(SEMI);
        for (i, c) in event.clean_text().enumerate() {
            if i > 0 {
                out.push(COLON);
            }
            push_decimal(out, u32::from(c));
        }
    }
    out.push(fin);
}

/// `:shifted:base`. The shifted key only when Shift is held and it differs;
/// the base-layout key only when it differs from the reported code.
fn write_alternates(event: &KeyEvent<'_>, number: u32, mods: Modifiers, out: &mut Vec<u8>) {
    let shifted = event
        .single_char()
        .map(u32::from)
        .filter(|&c| mods.contains(Modifiers::SHIFT) && c != number);
    let base = match event.key {
        Key::Char(base) => Some(u32::from(base)).filter(|&b| b != number),
        _ => None,
    };
    if shifted.is_none() && base.is_none() {
        return;
    }
    out.push(COLON);
    if let Some(shifted) = shifted {
        push_decimal(out, shifted);
    }
    if let Some(base) = base {
        out.push(COLON);
        push_decimal(out, base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::tests::{ALT, CTRL, CTRL_SHIFT, NONE, SHIFT, event};
    use crate::key::{KeypadKey, MediaKey, ModifierKey};

    const D: KittyFlags = KittyFlags::DISAMBIGUATE;
    const EVENTS: KittyFlags = D.union(KittyFlags::REPORT_EVENTS);
    const ALL: KittyFlags = KittyFlags::REPORT_ALL_KEYS;
    const ALL_TEXT: KittyFlags = ALL.union(KittyFlags::REPORT_TEXT);
    const ALTERNATES: KittyFlags = D.union(KittyFlags::REPORT_ALTERNATES);

    fn encode_with(event: &KeyEvent<'_>, flags: KittyFlags) -> String {
        let mut out = Vec::new();
        encode(event, flags, &mut out);
        String::from_utf8(out).unwrap()
    }

    fn enc(flags: KittyFlags, key: Key, mods: Modifiers, text: &str) -> String {
        encode_with(&event(key, mods, text), flags)
    }

    fn with_action(flags: KittyFlags, key: Key, mods: Modifiers, action: KeyAction) -> String {
        let mut e = event(key, mods, "");
        e.action = action;
        encode_with(&e, flags)
    }

    #[test]
    fn disambiguate_reports_esc_and_modified_text_keys_as_csi_u() {
        let table = [
            (Key::Escape, NONE, "", "\x1b[27u"),
            (Key::Escape, SHIFT, "", "\x1b[27;2u"),
            (Key::Char('a'), CTRL, "a", "\x1b[97;5u"),
            (Key::Char('a'), ALT, "a", "\x1b[97;3u"),
            (Key::Char('a'), CTRL_SHIFT, "A", "\x1b[97;6u"),
            (Key::Char('['), ALT, "[", "\x1b[91;3u"),
            (Key::Char(' '), CTRL, " ", "\x1b[32;5u"),
            (Key::Char('a'), Modifiers::SUPER, "a", "\x1b[97;9u"),
        ];
        for (key, mods, text, expected) in table {
            assert_eq!(enc(D, key, mods, text), expected, "{key:?} {mods:?}");
        }
    }

    #[test]
    fn disambiguate_keeps_text_and_enter_tab_backspace_legacy() {
        assert_eq!(enc(D, Key::Char('a'), NONE, "a"), "a");
        assert_eq!(enc(D, Key::Char('a'), SHIFT, "A"), "A");
        assert_eq!(enc(D, Key::Char('a'), Modifiers::CAPS_LOCK, "A"), "A");
        assert_eq!(enc(D, Key::Enter, NONE, ""), "\r");
        assert_eq!(enc(D, Key::Tab, NONE, ""), "\t");
        assert_eq!(enc(D, Key::Backspace, NONE, ""), "\x7f");
        // Modified, they become reports (fixterms erratum: Shift+Tab is 9;2).
        assert_eq!(enc(D, Key::Enter, SHIFT, ""), "\x1b[13;2u");
        assert_eq!(enc(D, Key::Tab, CTRL_SHIFT, ""), "\x1b[9;6u");
        assert_eq!(enc(D, Key::Backspace, CTRL, ""), "\x1b[127;5u");
    }

    #[test]
    fn functional_keys_use_the_functional_table_and_ignore_cursor_mode() {
        let table = [
            (Key::Up, NONE, "\x1b[A"),
            (Key::Up, CTRL, "\x1b[1;5A"),
            (Key::Home, SHIFT, "\x1b[1;2H"),
            (Key::F(1), NONE, "\x1b[P"),
            (Key::F(1), ALT, "\x1b[1;3P"),
            (Key::F(3), NONE, "\x1b[13~"),
            (Key::F(5), CTRL, "\x1b[15;5~"),
            (Key::Insert, NONE, "\x1b[2~"),
            (Key::F(13), NONE, "\x1b[57376u"),
            (Key::Media(MediaKey::Play), NONE, "\x1b[57428u"),
            (Key::System(SystemKey::PrintScreen), NONE, "\x1b[57361u"),
            (Key::System(SystemKey::Menu), NONE, "\x1b[57363u"),
            (Key::Keypad(KeypadKey::Enter), NONE, "\x1b[57414u"),
            (Key::Keypad(KeypadKey::Left), NONE, "\x1b[57417u"),
            (Key::Keypad(KeypadKey::Begin), NONE, "\x1b[E"),
            (Key::Keypad(KeypadKey::Digit1), CTRL, "\x1b[57400;5u"),
        ];
        for (key, mods, expected) in table {
            assert_eq!(enc(D, key, mods, ""), expected, "{key:?} {mods:?}");
        }
        assert_eq!(enc(D, Key::Keypad(KeypadKey::Digit1), NONE, "1"), "1");
    }

    #[test]
    fn lock_states_are_reported_for_functional_keys_only() {
        let num_lock = Modifiers::NUM_LOCK;
        assert_eq!(enc(D, Key::Up, num_lock, ""), "\x1b[1;129A");
        assert_eq!(enc(D, Key::Char('a'), num_lock | CTRL, "a"), "\x1b[97;5u");
        assert_eq!(
            enc(ALL, Key::Char('a'), Modifiers::CAPS_LOCK, "A"),
            "\x1b[97;65u"
        );
    }

    #[test]
    fn modifier_keys_are_reported_only_with_all_keys() {
        let shift = Key::Modifier(ModifierKey::LeftShift);
        assert_eq!(enc(D, shift, SHIFT, ""), "");
        assert_eq!(enc(D, Key::System(SystemKey::CapsLock), NONE, ""), "");
        assert_eq!(enc(ALL, shift, SHIFT, ""), "\x1b[57441;2u");
        let release = with_action(
            ALL | KittyFlags::REPORT_EVENTS,
            shift,
            NONE,
            KeyAction::Release,
        );
        assert_eq!(release, "\x1b[57441;1:3u");
    }

    #[test]
    fn event_types_are_reported_as_a_modifier_sub_field() {
        let table = [
            (Key::Up, NONE, KeyAction::Press, "\x1b[A"),
            (Key::Up, NONE, KeyAction::Repeat, "\x1b[1;1:2A"),
            (Key::Up, NONE, KeyAction::Release, "\x1b[1;1:3A"),
            (Key::Escape, NONE, KeyAction::Release, "\x1b[27;1:3u"),
            (Key::Char('a'), CTRL, KeyAction::Release, "\x1b[97;5:3u"),
            (Key::F(5), SHIFT, KeyAction::Repeat, "\x1b[15;2:2~"),
            // Text keys and Enter/Tab/Backspace have no release without all keys.
            (Key::Char('a'), NONE, KeyAction::Release, ""),
            (Key::Char('a'), SHIFT, KeyAction::Release, ""),
            (Key::Enter, NONE, KeyAction::Release, ""),
            (Key::Tab, CTRL, KeyAction::Release, ""),
        ];
        for (key, mods, action, expected) in table {
            let got = with_action(EVENTS, key, mods, action);
            assert_eq!(got, expected, "{key:?} {mods:?} {action:?}");
        }
        let mut repeat = event(Key::Char('a'), NONE, "a");
        repeat.action = KeyAction::Repeat;
        assert_eq!(encode_with(&repeat, EVENTS), "a");
    }

    #[test]
    fn releases_are_dropped_without_report_events() {
        assert_eq!(with_action(D, Key::Up, NONE, KeyAction::Release), "");
        assert_eq!(with_action(D, Key::Up, NONE, KeyAction::Repeat), "\x1b[A");
    }

    #[test]
    fn report_all_keys_turns_every_key_into_a_report() {
        let table = [
            (Key::Char('a'), NONE, "a", "\x1b[97u"),
            (Key::Char('a'), SHIFT, "A", "\x1b[97;2u"),
            (Key::Enter, NONE, "", "\x1b[13u"),
            (Key::Tab, NONE, "", "\x1b[9u"),
            (Key::Backspace, NONE, "", "\x1b[127u"),
            (Key::Escape, NONE, "", "\x1b[27u"),
            (Key::Char(' '), NONE, " ", "\x1b[32u"),
        ];
        for (key, mods, text, expected) in table {
            assert_eq!(enc(ALL, key, mods, text), expected, "{key:?} {mods:?}");
        }
        let mut release = event(Key::Enter, NONE, "");
        release.action = KeyAction::Release;
        assert_eq!(
            encode_with(&release, ALL | KittyFlags::REPORT_EVENTS),
            "\x1b[13;1:3u"
        );
    }

    #[test]
    fn associated_text_rides_in_the_third_field() {
        let table = [
            (Key::Char('a'), NONE, "a", "\x1b[97;;97u"),
            (Key::Char('a'), SHIFT, "A", "\x1b[97;2;65u"),
            (Key::Char('e'), NONE, "é", "\x1b[101;;233u"),
            (Key::Char('f'), NONE, "fi", "\x1b[102;;102:105u"),
            // Control characters are never text.
            (Key::Enter, NONE, "\r", "\x1b[13u"),
        ];
        for (key, mods, text, expected) in table {
            assert_eq!(enc(ALL_TEXT, key, mods, text), expected, "{key:?} {mods:?}");
        }
        let mut consumed_alt = event(Key::Char('a'), ALT, "å");
        consumed_alt.consumed = ALT;
        assert_eq!(encode_with(&consumed_alt, ALL_TEXT), "\x1b[97;;229u");
        // Text needs report-all; on its own it changes nothing.
        let text_only = D | KittyFlags::REPORT_TEXT;
        assert_eq!(enc(text_only, Key::Char('a'), CTRL, "a"), "\x1b[97;5u");
        let mut release = event(Key::Char('a'), NONE, "a");
        release.action = KeyAction::Release;
        let all_events = ALL_TEXT | KittyFlags::REPORT_EVENTS;
        assert_eq!(encode_with(&release, all_events), "\x1b[97;1:3u");
    }

    #[test]
    fn alternate_keys_report_shifted_and_base_layout_keys() {
        assert_eq!(
            enc(ALTERNATES, Key::Char('a'), CTRL_SHIFT, "A"),
            "\x1b[97:65;6u"
        );
        assert_eq!(enc(ALTERNATES, Key::Char('a'), CTRL, "a"), "\x1b[97;5u");
        assert_eq!(enc(ALTERNATES, Key::Char('a'), NONE, "a"), "a");
        // Cyrillic layout: key с (1089) sits on the US c (99).
        let mut cyrillic = event(Key::Char('c'), CTRL, "с");
        cyrillic.unshifted = Some('с');
        assert_eq!(encode_with(&cyrillic, ALTERNATES), "\x1b[1089::99;5u");
        cyrillic.mods = CTRL_SHIFT;
        cyrillic.text = "С";
        assert_eq!(encode_with(&cyrillic, ALTERNATES), "\x1b[1089:1057:99;6u");
        // Shift+= types + on a US layout: matches ctrl+plus shortcuts.
        let all_alt = ALL | KittyFlags::REPORT_ALTERNATES;
        assert_eq!(enc(all_alt, Key::Char('='), SHIFT, "+"), "\x1b[61:43;2u");
        // Functional keys have no alternates.
        assert_eq!(enc(ALTERNATES, Key::Up, SHIFT, ""), "\x1b[1;2A");
    }

    #[test]
    fn stack_push_pop_and_query_follow_the_specification() {
        let mut stack = KittyFlagStack::default();
        assert_eq!(stack.current(), KittyFlags::empty());
        stack.push(1);
        stack.push(0b11);
        assert_eq!(stack.current().bits(), 0b11);
        stack.pop(0);
        assert_eq!(stack.current(), D);
        stack.pop(1);
        assert_eq!(stack.current(), KittyFlags::empty());
        stack.pop(1);
        assert_eq!(stack.depth(), 0);
        stack.push(0b101);
        let mut reply = Vec::new();
        stack.write_query_reply(&mut reply);
        assert_eq!(reply, b"\x1b[?5u");
    }

    #[test]
    fn popping_more_than_the_stack_holds_resets_all_flags() {
        let mut stack = KittyFlagStack::default();
        stack.push(1);
        stack.push(2);
        stack.pop(u16::MAX);
        assert_eq!(stack.current(), KittyFlags::empty());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn full_stack_evicts_the_oldest_entry_and_never_grows() {
        let mut stack = KittyFlagStack::default();
        let pushes = u16::try_from(KITTY_FLAG_STACK_DEPTH).unwrap() + 2;
        for flags in 0..pushes {
            stack.push(flags % 32);
            assert!(stack.depth() <= KITTY_FLAG_STACK_DEPTH);
        }
        assert_eq!(stack.depth(), KITTY_FLAG_STACK_DEPTH);
        stack.pop(u16::try_from(KITTY_FLAG_STACK_DEPTH - 1).unwrap());
        // Entries 0 and 1 were evicted, so the bottom is now entry 2.
        assert_eq!(stack.current().bits(), 2);
        for _ in 0..10_000 {
            stack.push(1);
        }
        assert_eq!(stack.depth(), KITTY_FLAG_STACK_DEPTH);
    }

    #[test]
    fn set_replaces_unions_or_clears_the_active_flags() {
        let mut stack = KittyFlagStack::default();
        stack.set(0b101, 0);
        assert_eq!(stack.current().bits(), 0b101);
        stack.set(0b010, 2);
        assert_eq!(stack.current().bits(), 0b111);
        stack.set(0b001, 3);
        assert_eq!(stack.current().bits(), 0b110);
        stack.set(0b001, 1);
        assert_eq!(stack.current().bits(), 0b001);
        stack.set(0b11111, 4);
        assert_eq!(stack.current().bits(), 0b001, "unknown mode is ignored");
        stack.pop(1);
        assert_eq!(stack.current(), KittyFlags::empty());
    }

    #[test]
    fn undefined_flag_bits_from_the_pty_are_dropped() {
        let mut stack = KittyFlagStack::default();
        stack.push(u16::MAX);
        assert_eq!(stack.current(), KittyFlags::all());
        assert_eq!(KittyFlags::from_param(0b10_0000), KittyFlags::empty());
    }

    #[test]
    fn main_and_alternate_screens_keep_separate_stacks() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.stack_mut(Screen::Alternate).push(0b11111);
        assert_eq!(keyboard.stack(Screen::Main).current(), KittyFlags::empty());
        keyboard.stack_mut(Screen::Main).push(1);
        keyboard.stack_mut(Screen::Alternate).clear();
        assert_eq!(keyboard.stack(Screen::Main).current(), D);
        assert_eq!(keyboard.stack(Screen::Alternate).depth(), 0);
    }

    #[test]
    fn only_the_escaping_flags_leave_legacy_mode() {
        assert!(KittyFlags::empty().is_legacy());
        assert!((KittyFlags::REPORT_ALTERNATES | KittyFlags::REPORT_TEXT).is_legacy());
        for flags in [D, KittyFlags::REPORT_EVENTS, ALL] {
            assert!(!flags.is_legacy(), "{flags:?}");
        }
    }
}
