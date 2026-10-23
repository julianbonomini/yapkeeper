//! The command palette: a query line searched live as the person types,
//! the sessions it matches, a preview of the selected one and key hints,
//! over a `Catalog` loaded once.
//! It owns every terminal decision (keys, layout, theme and the terminal's
//! lifecycle) and never touches the index, the preferences or the disk.

mod terminal;
mod view;

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{Catalog, LeftOut, Listed, Message, Query, line};

pub use terminal::{Restore, run};

/// The command palette over a catalog: an empty query, every session it
/// lists, the first one selected.
pub struct Palette<'c> {
    catalog: &'c Catalog,
    colour: Colour,
    /// Shown as `~` at the start of a folder's path.
    home: Option<PathBuf>,
    query: String,
    /// `query` as the last search read it.
    parsed: Query,
    /// What the query found, in `search`'s order.
    found: Vec<&'c Listed>,
    left_out: LeftOut,
    selection: ListState,
    /// What was said in the selected session, for the preview.
    conversation: Vec<Message>,
    /// Why the last search, or the selected session's conversation, failed
    /// to read; cleared by the next search that succeeds. After a failed
    /// search the rows are the last ones that didn't fail.
    error: Option<String>,
}

/// Whether the palette is still open after a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    /// The person asked to leave; the caller closes the palette.
    Quit,
}

/// `Sage` accent with a grey selection bar, or `Mono` with no colour at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Sage,
    Mono,
}

impl Colour {
    /// `Mono` when `NO_COLOR` is set to a non-empty value, otherwise `Sage`.
    pub fn from_env() -> Colour {
        match std::env::var_os("NO_COLOR") {
            Some(value) if !value.is_empty() => Colour::Mono,
            _ => Colour::Sage,
        }
    }
}

impl<'c> Palette<'c> {
    /// The palette over `catalog`, drawn in `colour`. A search error is
    /// shown in the footer rather than returned.
    pub fn new(catalog: &'c Catalog, colour: Colour) -> Palette<'c> {
        let mut palette = Palette {
            catalog,
            colour,
            home: line::home(),
            query: String::new(),
            parsed: Query::default(),
            found: Vec::new(),
            left_out: LeftOut::default(),
            selection: ListState::default(),
            conversation: Vec::new(),
            error: None,
        };
        palette.search();
        palette
    }

    /// Applies one key press and says whether the palette is still open.
    /// Key releases and repeats are ignored.
    pub fn handle(&mut self, key: KeyEvent) -> Step {
        if key.kind != KeyEventKind::Press {
            return Step::Continue;
        }
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Char('c') if ctrl => return Step::Quit,
            KeyCode::Char('u') if ctrl => self.set_query(String::new()),
            KeyCode::Char('p') if ctrl => self.move_selection(Direction::Up),
            KeyCode::Char('n') if ctrl => self.move_selection(Direction::Down),
            KeyCode::Up => self.move_selection(Direction::Up),
            KeyCode::Down => self.move_selection(Direction::Down),
            KeyCode::Esc if self.query.is_empty() => return Step::Quit,
            KeyCode::Esc => self.set_query(String::new()),
            KeyCode::Backspace => {
                let mut query = self.query.clone();
                query.pop();
                self.set_query(query);
            }
            KeyCode::Char(c) if (key.modifiers - KeyModifiers::SHIFT).is_empty() => {
                self.set_query(format!("{}{c}", self.query));
            }
            _ => {}
        }
        Step::Continue
    }

    /// Draws the palette, or the too-small message, into the whole frame.
    pub fn render(&mut self, frame: &mut Frame) {
        view::render(self, frame);
    }

    fn set_query(&mut self, query: String) {
        if query != self.query {
            self.query = query;
            self.search();
        }
    }

    /// Searches the catalog for the query and selects the first row.
    fn search(&mut self) {
        self.parsed = self.catalog.parse(&self.query);
        match self.catalog.search(&self.parsed) {
            Ok(found) => {
                self.found = found.sessions;
                self.left_out = found.left_out;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        let first = (!self.found.is_empty()).then_some(0);
        self.selection = ListState::default();
        self.select(first);
    }

    /// Moves the selection one row, stopping at the first and last rows.
    fn move_selection(&mut self, direction: Direction) {
        let Some(selected) = self.selection.selected() else {
            return;
        };
        let last = self.found.len().saturating_sub(1);
        let row = match direction {
            Direction::Up => selected.saturating_sub(1),
            Direction::Down => (selected + 1).min(last),
        };
        if row != selected {
            self.select(Some(row));
        }
    }

    /// Selects `row` and reads what was said in its session. A read error
    /// is shown in the footer, with an empty conversation.
    fn select(&mut self, row: Option<usize>) {
        self.selection.select(row);
        let session = row.map(|row| &self.found[row].session);
        self.conversation = match session.map(|session| self.catalog.conversation(session)) {
            Some(Ok(conversation)) => conversation,
            Some(Err(error)) => {
                self.error = Some(error.to_string());
                Vec::new()
            }
            None => Vec::new(),
        };
    }

    /// The session the selection bar is on.
    fn selected(&self) -> Option<&'c Listed> {
        self.selection.selected().map(|row| self.found[row])
    }
}

enum Direction {
    Up,
    Down,
}
