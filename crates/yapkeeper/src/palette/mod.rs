//! The command palette: a query line searched live as the person types,
//! the sessions it matches or the ways to finish the filter being typed, a
//! preview of the selected session and key hints, over a `Catalog` loaded
//! once; or, in their place, the selected session's whole conversation.
//! It owns every terminal decision (keys, layout, theme and the terminal's
//! lifecycle) and reaches the index and the preferences only through the
//! catalog.

mod reading;
mod terminal;
mod view;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

use self::reading::Reading;
use crate::{Catalog, Completion, LeftOut, Listed, Message, Query, line, location_token};

pub use terminal::{Restore, run};

/// The command palette over a catalog: an empty query, every session it
/// lists, the first one selected.
pub struct Palette {
    catalog: Catalog,
    colour: Colour,
    /// Shown as `~` at the start of a folder's path.
    home: Option<PathBuf>,
    /// The `@` token `Ctrl+L` adds: where the palette was launched.
    launch_token: Option<String>,
    query: String,
    /// Whether searches also read tool calls, tool output and subagent
    /// transcripts.
    tools: bool,
    /// `query` as the last search read it.
    parsed: Query,
    /// What the query found, in `search`'s order.
    found: Vec<Rc<Listed>>,
    left_out: LeftOut,
    selection: ListState,
    /// The ways to finish the filter at the end of `query`; while there are
    /// any they take the results' place and the arrows move among them.
    completions: Vec<Completion>,
    completion: ListState,
    /// What was said in the selected session, with the tools called, for
    /// the preview and for reading.
    conversation: Vec<Message>,
    /// While the selected session's whole conversation is open, in place of
    /// the query, the results and the preview, which stay as they were.
    reading: Option<Reading>,
    /// Why the last search, or the selected session's conversation, failed
    /// to read; cleared by the next search that succeeds. After a failed
    /// search the rows are the last ones that didn't fail.
    error: Option<String>,
    /// Shown in the footer in place of the key hints.
    toast: Option<Toast>,
}

/// How long a confirmation stays in the footer.
const TOAST_TIME: Duration = Duration::from_secs(3);

/// A confirmation of the last action, shown until `until`.
struct Toast {
    said: Confirmation,
    until: Instant,
}

/// What a toast confirms.
enum Confirmation {
    /// The session with this title is hidden.
    Hid(String),
    /// The session with this title is shown again.
    Unhid(String),
    /// Searches read tool calls too, or no longer do.
    Tools(bool),
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

impl Palette {
    /// The palette over `catalog`, drawn in `colour`, launched from
    /// `launch_dir`. A search error is shown in the footer rather than
    /// returned.
    pub fn new(catalog: Catalog, colour: Colour, launch_dir: &Path) -> Palette {
        let mut palette = Palette {
            catalog,
            colour,
            home: line::home(),
            launch_token: location_token(launch_dir),
            query: String::new(),
            tools: false,
            parsed: Query::default(),
            found: Vec::new(),
            left_out: LeftOut::default(),
            selection: ListState::default(),
            completions: Vec::new(),
            completion: ListState::default(),
            conversation: Vec::new(),
            reading: None,
            error: None,
            toast: None,
        };
        palette.search(0);
        palette
    }

    /// Applies one key press, made at `now`, and says whether the palette
    /// is still open. Key releases and repeats are ignored.
    pub fn handle(&mut self, key: KeyEvent, now: Instant) -> Step {
        if key.kind != KeyEventKind::Press {
            return Step::Continue;
        }
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        if let Some(reading) = &mut self.reading {
            match key.code {
                KeyCode::Char('c') if ctrl => return Step::Quit,
                KeyCode::Esc | KeyCode::Left => self.reading = None,
                _ => reading.handle(key),
            }
            return Step::Continue;
        }
        match key.code {
            KeyCode::Char('c') if ctrl => return Step::Quit,
            KeyCode::Char('u') if ctrl => self.set_query(String::new()),
            KeyCode::Char('l') if ctrl => self.insert_launch_location(),
            KeyCode::Char('x') if ctrl => self.toggle_hidden(now),
            KeyCode::Char('t') if ctrl => self.toggle_tools(now),
            KeyCode::Char('p') if ctrl => self.move_selection(Direction::Up),
            KeyCode::Char('n') if ctrl => self.move_selection(Direction::Down),
            KeyCode::Up => self.move_selection(Direction::Up),
            KeyCode::Down => self.move_selection(Direction::Down),
            KeyCode::Right => self.open_reading(),
            KeyCode::Tab | KeyCode::Enter => {
                let chosen = self.completion.selected().map(|row| &self.completions[row]);
                match chosen {
                    Some(chosen) => self.set_query(chosen.query.clone()),
                    None if key.code == KeyCode::Tab => self.open_locations(),
                    None => {}
                }
            }
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

    /// Draws the palette as it is at `now`, or the too-small message, into
    /// the whole frame.
    pub fn render(&mut self, frame: &mut Frame, now: Instant) {
        if self.toast.as_ref().is_some_and(|toast| toast.until <= now) {
            self.toast = None;
        }
        view::render(self, frame);
    }

    /// When the frame changes with no key pressed: when the toast drawn
    /// last gives way to the key hints. `None` while nothing will.
    pub fn redraw_at(&self) -> Option<Instant> {
        self.toast.as_ref().map(|toast| toast.until)
    }

    fn set_query(&mut self, query: String) {
        if query != self.query {
            self.query = query;
            self.search(0);
            self.completions = self.catalog.completions(&self.query);
            let first = (!self.completions.is_empty()).then_some(0);
            self.completion = ListState::default().with_selected(first);
        }
    }

    /// Adds the `@` token for where the palette was launched to the end of
    /// the query; nothing when that place has no token.
    fn insert_launch_location(&mut self) {
        let Some(token) = &self.launch_token else {
            return;
        };
        let query = match self.query.trim_end() {
            "" => format!("{token} "),
            typed => format!("{typed} {token} "),
        };
        self.set_query(query);
    }

    /// Starts an `@` token at the end of the query, which opens the location
    /// list; nothing when the query already ends in a bare `@`, so a second
    /// `Tab` over an empty list types no second one.
    fn open_locations(&mut self) {
        if self.query.split_whitespace().next_back() == Some("@") {
            return;
        }
        let query = match self.query.is_empty() || self.query.ends_with(char::is_whitespace) {
            true => format!("{}@", self.query),
            false => format!("{} @", self.query),
        };
        self.set_query(query);
    }

    /// Hides the selected session, or shows it again when it is hidden, and
    /// confirms it. The selection stays on the same row, which now holds the
    /// next session. Nothing while completions are open, or when no session
    /// is selected; a failure to record it is shown in the footer.
    fn toggle_hidden(&mut self, now: Instant) {
        let Some(listed) = self.selected().filter(|_| self.completions.is_empty()) else {
            return;
        };
        let hidden = !listed.hidden;
        if let Err(error) = self.catalog.set_hidden(&listed.session, hidden) {
            self.error = Some(error.to_string());
            return;
        }
        let title = listed.session.title.clone();
        let said = match hidden {
            true => Confirmation::Hid(title),
            false => Confirmation::Unhid(title),
        };
        self.confirm(said, now);
        self.search(self.selection.selected().unwrap_or(0));
    }

    /// Searches tool calls too, or no longer does, from the first row, and
    /// confirms it.
    fn toggle_tools(&mut self, now: Instant) {
        self.tools = !self.tools;
        self.confirm(Confirmation::Tools(self.tools), now);
        self.search(0);
    }

    /// Shows `said` in the footer from `now` for `TOAST_TIME`.
    fn confirm(&mut self, said: Confirmation, now: Instant) {
        self.toast = Some(Toast {
            said,
            until: now + TOAST_TIME,
        });
    }

    /// Opens the selected session's whole conversation; nothing while
    /// completions are open, or when no session is selected.
    fn open_reading(&mut self) {
        if self.completions.is_empty() && self.selected().is_some() {
            self.reading = Some(Reading::new());
        }
    }

    /// Searches the catalog for the query and selects `row`, or the last row
    /// when there are fewer.
    fn search(&mut self, row: usize) {
        self.parsed = self.catalog.parse(&self.query).including_tools(self.tools);
        match self.catalog.search(&self.parsed) {
            Ok(found) => {
                self.found = found.sessions;
                self.left_out = found.left_out;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        let row = self.found.len().checked_sub(1).map(|last| row.min(last));
        self.selection = ListState::default();
        self.select(row);
    }

    /// Moves the selection one row, among the completions while there are
    /// any, otherwise among the sessions, stopping at the first and last rows.
    fn move_selection(&mut self, direction: Direction) {
        if self.completions.is_empty() {
            let row = direction.step(self.selection.selected(), self.found.len());
            if row != self.selection.selected() {
                self.select(row);
            }
        } else {
            let row = direction.step(self.completion.selected(), self.completions.len());
            self.completion.select(row);
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
    fn selected(&self) -> Option<Rc<Listed>> {
        self.selection
            .selected()
            .map(|row| Rc::clone(&self.found[row]))
    }
}

enum Direction {
    Up,
    Down,
}

impl Direction {
    /// The row one step from `selected` among `rows`, stopping at the first
    /// and last; nothing when nothing is selected.
    fn step(self, selected: Option<usize>, rows: usize) -> Option<usize> {
        selected.map(|row| match self {
            Direction::Up => row.saturating_sub(1),
            Direction::Down => (row + 1).min(rows.saturating_sub(1)),
        })
    }
}
