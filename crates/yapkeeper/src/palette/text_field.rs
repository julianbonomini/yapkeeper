//! A one-line text field: its text, where the cursor sits in it, the keys
//! that move the cursor and edit at it, and which part of the text shows
//! when it is wider than the room it has. It knows nothing of what the
//! text means; the palette reads it as a query.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// An empty field, the cursor at its start.
#[derive(Default)]
pub(super) struct TextField {
    text: String,
    /// The cursor's byte offset in `text`, always where a character starts
    /// or at the end.
    cursor: usize,
    /// Where the part `visible` last showed starts. Edits don't keep it on
    /// a character, so `visible` moves it back to one before reading it.
    scroll: usize,
}

/// What `TextField::visible` shows of the text.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Visible {
    /// The byte range of the text that shows.
    pub(super) range: Range<usize>,
    /// Whether text is left out before `range`; a `…` shows in its place.
    pub(super) clipped_start: bool,
    /// Whether text is left out after `range`; a `…` shows in its place.
    pub(super) clipped_end: bool,
    /// How many cells from the field's left edge the cursor sits, past
    /// the starting `…` when there is one.
    pub(super) cursor_cells: usize,
}

impl TextField {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text with `text`, without its control characters, and
    /// puts the cursor at its end.
    pub(super) fn set(&mut self, text: &str) {
        self.text = text.chars().filter(|c| !c.is_control()).collect();
        self.cursor = self.text.len();
    }

    /// Applies `key` if it is one a text field takes, and says whether the
    /// text changed. Typing inserts at the cursor; `←` `→` move it by a
    /// character, `Alt+←` `Alt+→` or `Alt+B` `Alt+F` by a word, `Home` `End`
    /// or `Ctrl+A` `Ctrl+E` to either end. `Backspace` and `Delete` delete a
    /// character before and after it, `Ctrl+W` or `Alt+Backspace` and
    /// `Alt+D` a word, and `Ctrl+U` and `Ctrl+K` everything before and after
    /// it. A word is a run of non-whitespace. A character is what reads as
    /// one: a combining mark, or an emoji joined by a zero-width joiner,
    /// goes with the character before it. Other keys change nothing.
    pub(super) fn handle(&mut self, key: KeyEvent) -> bool {
        // Terminals send `Alt+key` as ESC then the key, which crossterm
        // reports as the key with `ALT`, so one match takes both forms.
        let (ctrl, alt) = (KeyModifiers::CONTROL, KeyModifiers::ALT);
        let (start, end) = (0, self.text.len());
        let target = match (key.code, key.modifiers) {
            (KeyCode::Char(c), modifiers)
                if (modifiers - KeyModifiers::SHIFT).is_empty() && !c.is_control() =>
            {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                return true;
            }
            (KeyCode::Left, KeyModifiers::NONE) => Target::Move(self.before(self.cursor)),
            (KeyCode::Right, KeyModifiers::NONE) => Target::Move(self.after(self.cursor)),
            (KeyCode::Left, m) | (KeyCode::Char('b'), m) if m == alt => {
                Target::Move(self.word_start())
            }
            (KeyCode::Right, m) | (KeyCode::Char('f'), m) if m == alt => {
                Target::Move(self.word_end())
            }
            (KeyCode::Home, KeyModifiers::NONE) => Target::Move(start),
            (KeyCode::Char('a'), m) if m == ctrl => Target::Move(start),
            (KeyCode::End, KeyModifiers::NONE) => Target::Move(end),
            (KeyCode::Char('e'), m) if m == ctrl => Target::Move(end),
            (KeyCode::Backspace, KeyModifiers::NONE) => Target::Delete(self.before(self.cursor)),
            (KeyCode::Delete, KeyModifiers::NONE) => Target::Delete(self.after(self.cursor)),
            (KeyCode::Char('w'), m) if m == ctrl => Target::Delete(self.word_start()),
            (KeyCode::Backspace, m) if m == alt => Target::Delete(self.word_start()),
            (KeyCode::Char('d'), m) if m == alt => Target::Delete(self.word_end()),
            (KeyCode::Char('u'), m) if m == ctrl => Target::Delete(start),
            (KeyCode::Char('k'), m) if m == ctrl => Target::Delete(end),
            _ => return false,
        };
        match target {
            Target::Move(at) => {
                self.cursor = at;
                false
            }
            Target::Delete(at) => {
                let range = at.min(self.cursor)..at.max(self.cursor);
                self.cursor = range.start;
                self.text.replace_range(range.clone(), "");
                !range.is_empty()
            }
        }
    }

    /// What shows of the text in `cells` terminal cells, with one more
    /// after them for the cursor at the end: all of it when it fits,
    /// otherwise a part that holds the cursor, moved no further than it
    /// must be since the last call.
    pub(super) fn visible(&mut self, cells: usize) -> Visible {
        let mut start = self.scroll.min(self.text.len());
        while !self.starts_character(start) {
            start -= 1;
        }
        start = start.min(self.cursor);
        while !self.holds_cursor(start, cells) {
            start = self.after(start);
        }
        // After a deletion, take back room the text no longer needs.
        while start > 0 {
            let earlier = self.before(start);
            let window = self.window(earlier, cells);
            if window.end < self.text.len() {
                break;
            }
            start = earlier;
        }
        self.scroll = start;
        let range = self.window(start, cells);
        let clipped_start = start > 0;
        Visible {
            cursor_cells: usize::from(clipped_start) + self.text[start..self.cursor].width(),
            clipped_start,
            clipped_end: range.end < self.text.len(),
            range,
        }
    }

    /// The byte range that shows in `cells` cells from `start`, leaving a
    /// cell for each `…` a cut needs.
    fn window(&self, start: usize, cells: usize) -> Range<usize> {
        let room = cells.saturating_sub(usize::from(start > 0));
        if self.text[start..].width() <= room {
            return start..self.text.len();
        }
        let room = room.saturating_sub(1);
        let mut end = start;
        while end < self.text.len() {
            let next = self.after(end);
            if self.text[start..next].width() > room {
                break;
            }
            end = next;
        }
        start..end
    }

    /// Whether the window from `start` shows the cursor: inside it, or just
    /// after it when it reaches the end of the text. From the cursor itself
    /// it always does, even with no room at all.
    fn holds_cursor(&self, start: usize, cells: usize) -> bool {
        let window = self.window(start, cells);
        start == self.cursor || self.cursor < window.end || window.end == self.text.len()
    }

    /// Where the character before `at` starts, or the start of the text.
    fn before(&self, at: usize) -> usize {
        (0..at)
            .rev()
            .find(|&at| self.starts_character(at))
            .unwrap_or(0)
    }

    /// Where the character after the one at `at` starts, or the end of the
    /// text.
    fn after(&self, at: usize) -> usize {
        (at + 1..self.text.len())
            .find(|&at| self.starts_character(at))
            .unwrap_or(self.text.len())
    }

    /// Whether a character, as a person reads one, starts at `at`: not a
    /// zero-width mark joined to the one before, nor what a zero-width
    /// joiner joins to it.
    fn starts_character(&self, at: usize) -> bool {
        if at == 0 || at >= self.text.len() {
            return true;
        }
        if !self.text.is_char_boundary(at) {
            return false;
        }
        let joined = self.text[..at].ends_with(ZERO_WIDTH_JOINER);
        let c = self.text[at..].chars().next().unwrap_or(' ');
        !joined && UnicodeWidthChar::width(c) != Some(0)
    }

    /// The start of the word the cursor is in or after, skipping the
    /// whitespace before the cursor.
    fn word_start(&self) -> usize {
        let before = self.text[..self.cursor].trim_end();
        before.rfind(char::is_whitespace).map_or(0, |at| {
            at + before[at..].chars().next().map_or(1, char::len_utf8)
        })
    }

    /// The end of the word the cursor is in or before, skipping the
    /// whitespace after the cursor.
    fn word_end(&self) -> usize {
        let after = &self.text[self.cursor..];
        let skipped = after.len() - after.trim_start().len();
        let word = &after[skipped..];
        self.cursor + skipped + word.find(char::is_whitespace).unwrap_or(word.len())
    }
}

/// What a key does to the cursor: moves it to an offset, or deletes from
/// it to an offset.
enum Target {
    Move(usize),
    Delete(usize),
}

const ZERO_WIDTH_JOINER: char = '\u{200d}';

#[cfg(test)]
mod tests {
    use super::*;

    /// A field holding `text`, the cursor at the `|` in it.
    fn field(text: &str) -> TextField {
        let at = text.find('|').expect("a | marks the cursor");
        let mut field = TextField::default();
        field.set(&text.replace('|', ""));
        while field.cursor > at {
            field.handle(key(KeyCode::Left, KeyModifiers::NONE));
        }
        field
    }

    impl TextField {
        /// The text with a `|` where the cursor is.
        fn marked(&self) -> String {
            let (before, after) = self.text.split_at(self.cursor);
            format!("{before}|{after}")
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// `text` after `code` with `modifiers`, the cursor marked, and whether
    /// the text changed.
    fn after(text: &str, code: KeyCode, modifiers: KeyModifiers) -> (String, bool) {
        let mut field = field(text);
        let changed = field.handle(key(code, modifiers));
        (field.marked(), changed)
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    #[test]
    fn set_puts_the_cursor_at_the_end_and_drops_control_characters() {
        let mut field = TextField::default();
        field.set("retry\u{1b}[31m #codex");
        assert_eq!(field.marked(), "retry[31m #codex|");
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        assert_eq!(
            after("retry |#codex", KeyCode::Char('@'), NONE),
            ("retry @|#codex".into(), true)
        );
        assert_eq!(
            after("|etry", KeyCode::Char('R'), KeyModifiers::SHIFT),
            ("R|etry".into(), true)
        );
        assert_eq!(
            after("a|b", KeyCode::Char('x'), CTRL),
            ("a|b".into(), false)
        );
    }

    #[test]
    fn arrows_move_by_one_character_and_stop_at_either_end() {
        assert_eq!(after("ab|c", KeyCode::Left, NONE), ("a|bc".into(), false));
        assert_eq!(after("ab|c", KeyCode::Right, NONE), ("abc|".into(), false));
        assert_eq!(after("|abc", KeyCode::Left, NONE), ("|abc".into(), false));
        assert_eq!(after("abc|", KeyCode::Right, NONE), ("abc|".into(), false));
    }

    #[test]
    fn home_end_and_ctrl_a_ctrl_e_jump_to_either_end() {
        for (code, modifiers) in [(KeyCode::Home, NONE), (KeyCode::Char('a'), CTRL)] {
            assert_eq!(after("ab|c d", code, modifiers), ("|abc d".into(), false));
        }
        for (code, modifiers) in [(KeyCode::End, NONE), (KeyCode::Char('e'), CTRL)] {
            assert_eq!(after("ab|c d", code, modifiers), ("abc d|".into(), false));
        }
    }

    #[test]
    fn both_forms_of_alt_jump_a_whole_token_at_a_time() {
        let query = "retry  @acme-api:billing-retry #codex";
        for (code, modifiers) in [(KeyCode::Left, ALT), (KeyCode::Char('b'), ALT)] {
            let back = |text| after(text, code, modifiers).0;
            assert_eq!(
                back("retry  @acme-api:billing-retry #co|dex"),
                "retry  @acme-api:billing-retry |#codex"
            );
            assert_eq!(
                back("retry  @acme-api:billing-retry |#codex"),
                "retry  |@acme-api:billing-retry #codex"
            );
            assert_eq!(
                back("retry  |@acme-api:billing-retry #codex"),
                "|retry  @acme-api:billing-retry #codex"
            );
            assert_eq!(back(&format!("|{query}")), format!("|{query}"));
        }
        for (code, modifiers) in [(KeyCode::Right, ALT), (KeyCode::Char('f'), ALT)] {
            let forward = |text| after(text, code, modifiers).0;
            assert_eq!(
                forward("re|try  @acme-api:billing-retry #codex"),
                "retry|  @acme-api:billing-retry #codex"
            );
            assert_eq!(
                forward("retry|  @acme-api:billing-retry #codex"),
                "retry  @acme-api:billing-retry| #codex"
            );
            assert_eq!(forward(&format!("{query}|")), format!("{query}|"));
        }
    }

    #[test]
    fn backspace_and_delete_remove_one_character_either_side() {
        assert_eq!(
            after("ab|c", KeyCode::Backspace, NONE),
            ("a|c".into(), true)
        );
        assert_eq!(after("ab|c", KeyCode::Delete, NONE), ("ab|".into(), true));
        assert_eq!(
            after("|abc", KeyCode::Backspace, NONE),
            ("|abc".into(), false)
        );
        assert_eq!(after("abc|", KeyCode::Delete, NONE), ("abc|".into(), false));
    }

    #[test]
    fn word_deletion_removes_a_whole_token() {
        for (code, modifiers) in [(KeyCode::Char('w'), CTRL), (KeyCode::Backspace, ALT)] {
            assert_eq!(
                after("retry @acme-api:wt |#codex", code, modifiers),
                ("retry |#codex".into(), true)
            );
            assert_eq!(after("ret|ry", code, modifiers), ("|ry".into(), true));
            assert_eq!(after("|retry", code, modifiers), ("|retry".into(), false));
        }
        assert_eq!(
            after("retry| @acme-api:wt #codex", KeyCode::Char('d'), ALT),
            ("retry| #codex".into(), true)
        );
        assert_eq!(
            after("retry|", KeyCode::Char('d'), ALT),
            ("retry|".into(), false)
        );
    }

    #[test]
    fn ctrl_u_deletes_to_the_start_and_ctrl_k_to_the_end() {
        assert_eq!(
            after("retry |#codex", KeyCode::Char('u'), CTRL),
            ("|#codex".into(), true)
        );
        assert_eq!(
            after("retry #codex|", KeyCode::Char('u'), CTRL),
            ("|".into(), true)
        );
        assert_eq!(
            after("retry |#codex", KeyCode::Char('k'), CTRL),
            ("retry |".into(), true)
        );
        assert_eq!(after("|", KeyCode::Char('u'), CTRL), ("|".into(), false));
    }

    #[test]
    fn a_combining_mark_moves_and_deletes_with_its_character() {
        let cafe = "cafe\u{301}";
        assert_eq!(
            after(&format!("{cafe}|"), KeyCode::Left, NONE).0,
            "caf|e\u{301}"
        );
        assert_eq!(
            after("caf|e\u{301}", KeyCode::Right, NONE).0,
            format!("{cafe}|")
        );
        assert_eq!(
            after(&format!("{cafe}|"), KeyCode::Backspace, NONE).0,
            "caf|"
        );
        assert_eq!(after("caf|e\u{301}", KeyCode::Delete, NONE).0, "caf|");
        let coder = "\u{1f469}\u{200d}\u{1f4bb}";
        assert_eq!(
            after(&format!("a{coder}|"), KeyCode::Left, NONE).0,
            format!("a|{coder}")
        );
        assert_eq!(
            after(&format!("a{coder}|"), KeyCode::Backspace, NONE).0,
            "a|"
        );
    }

    #[test]
    fn wide_characters_move_and_delete_whole_and_count_two_cells() {
        assert_eq!(after("日本|語", KeyCode::Left, NONE).0, "日|本語");
        assert_eq!(after("日本|語", KeyCode::Backspace, NONE).0, "日|語");
        assert_eq!(after("日本 語|", KeyCode::Char('w'), CTRL).0, "日本 |");
        let mut field = field("日本|語");
        assert_eq!(field.visible(20).cursor_cells, 4);
    }

    #[test]
    fn a_text_that_fits_shows_whole() {
        let mut field = field("ab|c");
        assert_eq!(
            field.visible(3),
            Visible {
                range: 0..3,
                clipped_start: false,
                clipped_end: false,
                cursor_cells: 2,
            }
        );
    }

    #[test]
    fn a_wide_text_scrolls_only_as_far_as_the_cursor_needs() {
        let mut field = field("0123456789|");
        // Cells 0 to 9 hold `…3456789`, and the cursor the one after.
        assert_eq!(
            field.visible(8),
            Visible {
                range: 3..10,
                clipped_start: true,
                clipped_end: false,
                cursor_cells: 8,
            }
        );
        // Moving left inside the window doesn't scroll it.
        for _ in 0..4 {
            field.handle(key(KeyCode::Left, NONE));
        }
        assert_eq!(field.visible(8).range, 3..10);
        // Moving past its start scrolls by as little as it takes.
        for _ in 0..4 {
            field.handle(key(KeyCode::Left, NONE));
        }
        assert_eq!(
            field.visible(8),
            Visible {
                range: 2..8,
                clipped_start: true,
                clipped_end: true,
                cursor_cells: 1,
            }
        );
        field.handle(key(KeyCode::Home, NONE));
        assert_eq!(
            field.visible(8),
            Visible {
                range: 0..7,
                clipped_start: false,
                clipped_end: true,
                cursor_cells: 0,
            }
        );
        field.handle(key(KeyCode::End, NONE));
        assert_eq!(field.visible(8).range, 3..10);
    }

    #[test]
    fn deleting_from_a_scrolled_text_shows_as_much_of_it_as_fits() {
        let mut field = field("0123456789|");
        field.visible(8);
        field.handle(key(KeyCode::Backspace, NONE));
        field.handle(key(KeyCode::Backspace, NONE));
        assert_eq!(field.visible(8).range, 0..8);
    }

    #[test]
    fn a_wide_character_is_never_split_by_the_window() {
        let mut field = field("日本語日本語|");
        let shown = field.visible(7);
        assert_eq!(&field.text()[shown.range.clone()], "日本語");
        assert!(shown.clipped_start);
        assert_eq!(shown.cursor_cells, 7);
    }
}
