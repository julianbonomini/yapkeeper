//! The command palette: a query line searched live as the person types, the
//! sessions it matches or the ways to finish the filter being typed, a
//! preview of the selected session and key hints, over a `Catalog` an
//! `Indexer` keeps up to date; or, in their place, the selected session's
//! whole conversation, a settings screen or the resume dialog. It owns
//! every terminal decision (keys, layout, theme and the terminal's
//! lifecycle) and reaches the index, the preferences, settings included,
//! and what resuming would do only through the catalog.

mod reading;
mod resuming;
mod settings;
mod terminal;
mod view;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

use self::reading::Reading;
use self::resuming::Resuming;
use self::settings::SettingsScreen;
use crate::Snapshot;
use crate::indexer::Indexing;
use crate::{
    Catalog, Completion, Indexer, LeftOut, Listed, Message, Query, ResumeCommand, SessionKey,
    Settings, line, location_token,
};

pub use terminal::{Restore, run};

/// The command palette over a catalog: an empty query, every session it
/// lists, the first one selected.
pub struct Palette {
    catalog: Catalog,
    colour: Colour,
    /// Shown as `~` at the start of a folder's path.
    home: Option<PathBuf>,
    /// Where the palette was launched; `c` in the resume dialog resumes here.
    launch_dir: PathBuf,
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
    /// While the settings are open, in place of the query, the results and
    /// the preview, which stay as they were.
    settings: Option<SettingsScreen>,
    /// While the resume dialog is open, in place of the query, the results
    /// and the preview, or of the conversation, which stay as they were.
    resuming: Option<Resuming>,
    /// Why the last search, or the selected session's conversation, failed
    /// to read; cleared by the next search that succeeds. After a failed
    /// search the rows are the last ones that didn't fail.
    error: Option<String>,
    /// Shown in the footer in place of the key hints.
    toast: Option<Toast>,
    /// What keeps `catalog` up to date; idle unless given by `indexing`.
    indexer: Indexer,
    /// The first render that found a pass running, until one finds it ended.
    running_since: Option<Instant>,
    /// When the last render drew.
    drawn_at: Option<Instant>,
    /// Whether the indexer published while the conversation or the resume
    /// dialog was open, so the results behind it are yet to be searched again.
    stale: bool,
}

/// How long a confirmation stays in the footer.
const TOAST_TIME: Duration = Duration::from_secs(3);

/// How long a pass runs before the query line says so, so a pass that
/// finds nothing to read never flashes it.
const INDEXING_SHOWN_AFTER: Duration = Duration::from_millis(150);

/// How often the palette redraws while a pass runs, to show what it publishes.
const INDEXING_TICK: Duration = Duration::from_millis(50);

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Continue,
    /// The person asked to leave; the caller closes the palette.
    Quit,
    /// The person confirmed resuming; the caller closes the palette,
    /// restores the terminal, then starts this.
    Resume(ResumeCommand),
}

/// `Accent` draws in the accent the settings choose; `Mono` draws no colour
/// whatever they choose, as `NO_COLOR` asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Accent,
    Mono,
}

impl Colour {
    /// `Mono` when `NO_COLOR` is set to a non-empty value, otherwise `Accent`.
    pub fn from_env() -> Colour {
        match std::env::var_os("NO_COLOR") {
            Some(value) if !value.is_empty() => Colour::Mono,
            _ => Colour::Accent,
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
            launch_dir: launch_dir.to_path_buf(),
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
            settings: None,
            resuming: None,
            error: None,
            toast: None,
            indexer: Indexer::idle(),
            running_since: None,
            drawn_at: None,
            stale: false,
        };
        palette.search(0);
        palette
    }

    /// This palette, taking in each batch `indexer` publishes.
    pub fn indexing(self, indexer: Indexer) -> Palette {
        Palette { indexer, ..self }
    }

    /// Applies one key press, made at `now`, and says whether the palette
    /// is still open. Key releases and repeats are ignored.
    pub fn handle(&mut self, key: KeyEvent, now: Instant) -> Step {
        if key.kind != KeyEventKind::Press {
            return Step::Continue;
        }
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        if let Some(resuming) = &mut self.resuming {
            match key.code {
                KeyCode::Char('c') if ctrl => return Step::Quit,
                KeyCode::Esc => {
                    self.resuming = None;
                    self.catch_up();
                }
                _ => {
                    if let Some(command) = resuming.handle(key, &self.catalog, &self.launch_dir) {
                        return Step::Resume(command);
                    }
                }
            }
            return Step::Continue;
        }
        if let Some(screen) = &mut self.settings {
            match key.code {
                KeyCode::Char('c') if ctrl => return Step::Quit,
                KeyCode::Char('s') if ctrl => self.settings = None,
                KeyCode::Esc => self.settings = None,
                _ => {
                    if let Some(changed) = screen.handle(key, self.catalog.settings()) {
                        self.change_settings(changed);
                    }
                }
            }
            return Step::Continue;
        }
        if let Some(reading) = &mut self.reading {
            match key.code {
                KeyCode::Char('c') if ctrl => return Step::Quit,
                KeyCode::Esc | KeyCode::Left => {
                    self.reading = None;
                    self.catch_up();
                }
                KeyCode::Enter => self.open_resuming(),
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
            KeyCode::Char('s') if ctrl => self.settings = Some(SettingsScreen::new()),
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
                    None => self.open_resuming(),
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
    /// the whole frame. First takes in the sessions the indexer has
    /// published by `now`. The query, the selected session when it is still
    /// listed, and the place in the conversation are kept.
    pub fn render(&mut self, frame: &mut Frame, now: Instant) {
        self.take_in(now);
        if self.toast.as_ref().is_some_and(|toast| toast.until <= now) {
            self.toast = None;
        }
        view::render(self, frame);
    }

    /// When the frame changes with no key pressed: when the toast gives way
    /// to the key hints, or 50 ms after the last render while a pass runs.
    /// `None` while nothing will.
    pub fn redraw_at(&self) -> Option<Instant> {
        let toast = self.toast.as_ref().map(|toast| toast.until);
        let tick = self
            .drawn_at
            .filter(|_| self.running_since.is_some())
            .map(|drawn_at| drawn_at + INDEXING_TICK);
        toast.into_iter().chain(tick).min()
    }

    /// Whether the query line says a pass is running: once it has run for
    /// `INDEXING_SHOWN_AFTER`.
    fn shows_indexing(&self) -> bool {
        match (self.running_since, self.drawn_at) {
            (Some(since), Some(drawn_at)) => drawn_at >= since + INDEXING_SHOWN_AFTER,
            _ => false,
        }
    }

    /// Takes in what the indexer has done by `now`: whether a pass runs, why
    /// it failed, and the sessions it published since the last render.
    fn take_in(&mut self, now: Instant) {
        self.drawn_at = Some(now);
        // Read before the snapshot, so the last batch of a pass that has just
        // ended is taken in by this render or the one its tick brings.
        let status = self.indexer.status();
        match status {
            Indexing::Running => {
                self.running_since.get_or_insert(now);
            }
            Indexing::Idle | Indexing::Failed(_) => self.running_since = None,
        }
        if let Some(snapshot) = self.indexer.take() {
            self.apply(snapshot);
        }
        // After the search the snapshot brings, which would clear it.
        if let Indexing::Failed(message) = status {
            self.error = Some(message);
        }
    }

    /// Takes `snapshot` into the catalog. An open conversation keeps its place
    /// and reads its messages again; the results it or the resume dialog
    /// hides wait for `catch_up`.
    fn apply(&mut self, snapshot: Snapshot) {
        self.catalog.apply(snapshot);
        if self.reading.is_none() && self.resuming.is_none() {
            self.refind();
            return;
        }
        self.stale = true;
        let read = self.selected().filter(|_| self.reading.is_some());
        if let Some(listed) = read.filter(|listed| self.catalog.holds(&listed.session)) {
            match self.catalog.conversation(&listed.session) {
                Ok(conversation) => self.conversation = conversation,
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    /// Searches again for what the indexer published while the conversation
    /// or the resume dialog was open, once neither is.
    fn catch_up(&mut self) {
        if self.stale && self.reading.is_none() && self.resuming.is_none() {
            self.stale = false;
            self.refind();
        }
    }

    /// Searches for the query again after the catalog changed, keeping the
    /// selected session when it is still found, else the same row, and the
    /// rows scrolled into view; the ways to finish the query keep the one
    /// selected when it is still offered, else the first.
    fn refind(&mut self) {
        let selected = self
            .selected()
            .map(|listed| SessionKey::of(&listed.session));
        let row = self.selection.selected().unwrap_or(0);
        let offset = self.selection.offset();
        let completion = self
            .completion
            .selected()
            .map(|row| self.completions[row].token.clone());
        self.find();
        let row = self
            .found
            .iter()
            .position(|listed| Some(SessionKey::of(&listed.session)) == selected)
            .or_else(|| self.found.len().checked_sub(1).map(|last| row.min(last)));
        self.selection = ListState::default().with_offset(offset);
        self.select(row);
        self.completions = self.catalog.completions(&self.query);
        let kept = self
            .completions
            .iter()
            .position(|offered| Some(&offered.token) == completion.as_ref());
        let first = (!self.completions.is_empty()).then_some(0);
        self.completion = ListState::default().with_selected(kept.or(first));
    }

    fn set_query(&mut self, query: String) {
        if query != self.query {
            self.query = query;
            self.reapply();
        }
    }

    /// Saves `settings` and shows what they change at once; a failure to
    /// save them is shown in the footer and changes nothing.
    fn change_settings(&mut self, settings: Settings) {
        match self.catalog.set_settings(settings) {
            Ok(()) => self.reapply(),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Searches for the query again from the first row and works out the
    /// ways to finish it, as after the query or the settings change.
    fn reapply(&mut self) {
        self.search(0);
        self.completions = self.catalog.completions(&self.query);
        let first = (!self.completions.is_empty()).then_some(0);
        self.completion = ListState::default().with_selected(first);
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

    /// Opens the resume dialog on the selected session; nothing when no
    /// session is selected.
    fn open_resuming(&mut self) {
        if let Some(listed) = self.selected() {
            self.resuming = Some(Resuming::open(&self.catalog, &listed.session));
        }
    }

    /// Searches the catalog for the query and selects `row`, or the last row
    /// when there are fewer.
    fn search(&mut self, row: usize) {
        self.find();
        let row = self.found.len().checked_sub(1).map(|last| row.min(last));
        self.selection = ListState::default();
        self.select(row);
    }

    /// Searches the catalog for the query, leaving the selection to the caller.
    fn find(&mut self) {
        self.parsed = self.catalog.parse(&self.query).including_tools(self.tools);
        match self.catalog.search(&self.parsed) {
            Ok(found) => {
                self.found = found.sessions;
                self.left_out = found.left_out;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
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
