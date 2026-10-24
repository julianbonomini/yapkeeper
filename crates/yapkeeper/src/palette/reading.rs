//! Where the person is while reading the selected session's whole
//! conversation: the scroll position and the match `n` and `N` move to.
//! Lines exist only once drawn at the frame's width, so keys record what
//! was asked and `settle` resolves it against the lines drawn.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Lines kept above a match the view scrolls to, so it reads in context.
const ABOVE_MATCH: usize = 2;

/// The conversation's scroll position and current match.
pub(super) struct Reading {
    top: usize,
    /// Lines a page key scrolls, as last settled.
    page: usize,
    /// Counted from the first match; `n` and `N` step it past either end
    /// and `settle` wraps it around.
    current: isize,
    /// Whether `settle` scrolls to the current match: on opening, and after
    /// `n` or `N`.
    reveal: bool,
}

/// What `Reading::settle` resolved.
pub(super) struct Settled {
    /// The first line to draw.
    pub(super) top: usize,
    /// The current match's index, when there are matches.
    pub(super) current: Option<usize>,
}

impl Reading {
    /// Reading from the first match, or from the start when nothing matches.
    pub(super) fn new() -> Reading {
        Reading {
            top: 0,
            page: 0,
            current: 0,
            reveal: true,
        }
    }

    /// Scrolls by a line with the arrows, by a page with the page keys, to
    /// either end with `Home` and `End`, and to the next or previous match
    /// with `n` and `N`, wrapping around. Other keys do nothing.
    pub(super) fn handle(&mut self, key: KeyEvent) {
        let plain = (key.modifiers - KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Up => self.top = self.top.saturating_sub(1),
            KeyCode::Down => self.top = self.top.saturating_add(1),
            KeyCode::PageUp => self.top = self.top.saturating_sub(self.page),
            KeyCode::PageDown => self.top = self.top.saturating_add(self.page),
            KeyCode::Home => self.top = 0,
            KeyCode::End => self.top = usize::MAX,
            KeyCode::Char('n') if plain => self.step_match(1),
            KeyCode::Char('N') if plain => self.step_match(-1),
            _ => {}
        }
    }

    fn step_match(&mut self, by: isize) {
        self.current += by;
        self.reveal = true;
    }

    /// Settles on `lines` lines drawn in a body `height` lines high, whose
    /// matches start on the lines in `hits`, in order. The last line never
    /// rises above the bottom of the body, and a match asked for shows near
    /// the top.
    pub(super) fn settle(&mut self, lines: usize, height: usize, hits: &[usize]) -> Settled {
        // A page keeps the line that was last at the bottom, for context.
        self.page = height.saturating_sub(1).max(1);
        let current = match hits.len() {
            0 => None,
            count => Some(self.current.rem_euclid(count as isize) as usize),
        };
        self.current = current.map_or(0, |current| current as isize);
        if let Some(current) = current.filter(|_| self.reveal) {
            self.top = hits[current].saturating_sub(ABOVE_MATCH);
        }
        self.reveal = false;
        self.top = self.top.min(lines.saturating_sub(height));
        Settled {
            top: self.top,
            current,
        }
    }
}
