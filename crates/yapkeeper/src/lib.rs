//! Sessions from every provider, merged and ordered, each with where it ran.
//! Each provider's format stays inside its adapter; this crate knows where
//! each one is rooted and where Yapkeeper keeps its own files, keeps the
//! index of every session up to date, finds sessions by what was said and
//! by the query language's filters, reads each session's location from
//! the disk when it is listed, keeps the user's hides and settings apart
//! from the index, and says what resuming a session would do now, and
//! starts it. A `Catalog` loads all of that from the index, and takes in
//! each batch an `Indexer` publishes, so it can be searched many times, by
//! the settings, and record hides and settings, as the `palette` does while
//! the person types; it can find sessions on a worker of its own, so typing
//! never waits on a search;
//! `line` holds how a session reads as one line of terminal text, for the
//! palette and the CLI.

#![forbid(unsafe_code)]

mod index;
mod indexer;
pub mod line;
mod location;
pub mod palette;
mod prefs;
mod query;
mod resume;
mod searcher;
mod settings;

use prefs::Choice;
use searcher::{Matched, Matcher, Searcher};

use std::cell::RefCell;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use yapkeeper_claude::ClaudeCode;
use yapkeeper_codex::Codex;
use yapkeeper_pi::Pi;

pub use indexer::{Indexer, REFRESH_INTERVAL, Stepper};
pub use location::*;
pub use query::{
    Completion, Edited, LocationRow, LocationRowKind, Query, State, Token, TokenKind,
    location_token, place_location, start_location,
};
pub use resume::{Caution, ResumeDir, ResumePlan, launch};
pub use searcher::SearchStepper;
pub use settings::*;
pub use yapkeeper_model::*;

/// Where each provider keeps its history.
#[derive(Clone)]
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub pi: PathBuf,
}

impl Roots {
    /// The roots this process's environment names. Fails only when no home
    /// directory can be found for a provider that needs one.
    pub fn from_env() -> Result<Roots, MissingHome> {
        Ok(Roots {
            claude: yapkeeper_claude::config_dir()?,
            codex: yapkeeper_codex::home_dir()?,
            pi: yapkeeper_pi::session_dir()?,
        })
    }

    /// The provider name of each adapter `search` reads, in search order,
    /// such as `["claude", "codex", "pi"]`. Opens nothing.
    pub fn providers(&self) -> Vec<&'static str> {
        adapters(self)
            .iter()
            .map(|adapter| adapter.provider())
            .collect()
    }

    /// Each adapter's provider name and history directory, in search order.
    /// Opens nothing.
    pub fn history_dirs(&self) -> Vec<HistoryDir> {
        adapters(self)
            .iter()
            .map(|adapter| HistoryDir {
                provider: adapter.provider(),
                dir: adapter.history_dir(),
            })
            .collect()
    }
}

/// One provider's history directory, as this environment resolves it.
pub struct HistoryDir {
    /// Such as "claude", the name `#provider` takes.
    pub provider: &'static str,
    pub dir: PathBuf,
}

/// An adapter for each provider, reading from `roots`.
fn adapters(roots: &Roots) -> [Box<dyn Adapter>; 3] {
    [
        Box::new(ClaudeCode::at(roots.claude.clone())),
        Box::new(Codex::at(roots.codex.clone())),
        Box::new(Pi::at(roots.pi.clone())),
    ]
}

/// Where Yapkeeper keeps its own files: `$XDG_DATA_HOME/yapkeeper` when that
/// is set and absolute, otherwise `$HOME/.local/share/yapkeeper`. Creates nothing.
#[derive(Clone)]
pub struct DataDir {
    dir: PathBuf,
}

impl DataDir {
    /// Fails with `MissingHome { variable: "XDG_DATA_HOME" }` when neither names a directory.
    pub fn from_env() -> Result<DataDir, MissingHome> {
        // The XDG spec says a relative path is invalid and must be ignored.
        let base = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            Some(dir) if dir.is_absolute() => dir,
            _ => match dir_from_env("HOME") {
                Some(home) => home.join(".local/share"),
                None => {
                    return Err(MissingHome {
                        variable: "XDG_DATA_HOME",
                    });
                }
            },
        };
        Ok(DataDir::at(base.join("yapkeeper")))
    }

    pub fn at(dir: PathBuf) -> DataDir {
        DataDir { dir }
    }

    /// `<dir>/index.sqlite`, rebuildable and safe to delete at any time.
    pub fn index(&self) -> PathBuf {
        self.dir.join("index.sqlite")
    }

    /// `<dir>/prefs.sqlite`, the user's hides and settings; never touched by
    /// the index.
    pub fn prefs(&self) -> PathBuf {
        self.dir.join("prefs.sqlite")
    }
}

/// Sessions from every provider, with what was skipped, where it looked and
/// what it read.
pub struct Listing {
    /// Orphaned last; each group most recent first.
    pub sessions: Vec<Listed>,
    /// Every session in the index, whether or not the query kept it.
    pub indexed: usize,
    /// Malformed complete lines in every indexed file, not just those read this call.
    pub malformed_lines: usize,
    /// Each provider's directory looked in, for the empty-state message.
    pub searched: Vec<PathBuf>,
    /// History files opened during this call; zero when nothing changed.
    pub files_read: usize,
    /// History bytes read during this call; only the new bytes of a file that grew.
    pub bytes_read: u64,
    /// How many sessions the query otherwise matched each default left out.
    pub left_out: LeftOut,
}

/// How many sessions every filter admitted that each default left out: temp
/// sessions unless the query has `is:temp` (none while `hide_temp` is off),
/// and hidden ones unless it has `is:hidden`. A session can count in both.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeftOut {
    pub temp: usize,
    pub hidden: usize,
}

/// A session with its location and whether it is hidden, as of this listing.
#[derive(Clone)]
pub struct Listed {
    pub session: Session,
    pub location: Location,
    pub hidden: bool,
}

/// One of Yapkeeper's own files, the index or the preferences at `path`,
/// can't be created, opened, read or written.
#[derive(Debug)]
pub struct DataError {
    pub path: PathBuf,
    file: DataFile,
    source: Cause,
}

/// How long the index or the preferences wait for another `yapkeeper`
/// process that is writing them.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Why one of Yapkeeper's own files failed.
type Cause = Box<dyn Error + Send + Sync>;

/// A session as the index and the preferences name it: by provider as well
/// as id, so two providers can never share one.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    provider: String,
    id: String,
}

impl SessionKey {
    fn of(session: &Session) -> SessionKey {
        SessionKey {
            provider: session.provider.to_string(),
            id: session.id.clone(),
        }
    }
}

#[derive(Debug)]
enum DataFile {
    Index,
    Preferences,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.file {
            DataFile::Index => "use the index",
            DataFile::Preferences => "use the preferences",
        };
        write!(
            f,
            "can't {what} at {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl Error for DataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Brings the index up to date, opening only history files that changed
/// since the last call, then returns every session in it outside temp
/// folders and not hidden, in `search`'s order, each with its location
/// read from the disk now. A missing, corrupt or old index is recreated.
/// Fails only when the index can't be created or written, or the
/// preferences can't be read.
pub fn list(roots: &Roots, data: &DataDir) -> Result<Listing, DataError> {
    search(roots, data, &Query::default())
}

/// Brings the index up to date as `list` does, then returns the sessions
/// that match every token of `query`. Temp sessions are left out unless
/// the query has a non-negated `is:temp`, and hidden ones unless it has
/// `is:hidden`. Orphaned sessions come after the rest, each group most
/// recent `last_active` first, ties by provider then id. The other fields
/// are as `list` gives them. Fails only as `list` does.
pub fn search(roots: &Roots, data: &DataDir, query: &Query) -> Result<Listing, DataError> {
    listing(&Catalog::open(roots, data)?, query)
}

/// What `catalog.search` finds for `query`, with what opening it read.
fn listing(catalog: &Catalog, query: &Query) -> Result<Listing, DataError> {
    let found = catalog.search(query)?;
    Ok(Listing {
        sessions: found
            .sessions
            .iter()
            .map(|listed| Listed::clone(listed))
            .collect(),
        indexed: catalog.indexed(),
        malformed_lines: catalog.malformed_lines,
        searched: catalog.searched.clone(),
        files_read: catalog.files_read,
        bytes_read: catalog.bytes_read,
        left_out: found.left_out,
    })
}

/// Every session in the index as of one read, each with its location read
/// from the disk then; `hidden` is left to the catalog that applies it.
pub(crate) struct Snapshot {
    sessions: Vec<Listed>,
    malformed_lines: usize,
}

impl Snapshot {
    /// `sessions`, each with its location read from the disk now.
    pub(crate) fn of(sessions: index::Sessions) -> Snapshot {
        Snapshot {
            sessions: sessions
                .sessions
                .into_iter()
                .map(|session| listed(session, false))
                .collect(),
            malformed_lines: sessions.malformed_lines,
        }
    }
}

/// What opening a catalog read from the histories, for its listings.
struct Opening {
    searched: Vec<PathBuf>,
    files_read: usize,
    bytes_read: u64,
}

/// Every indexed session as of the catalog's last snapshot, with its
/// location and whether it is hidden, ready to be searched many times
/// without refreshing the index or rereading the disk, to record hides and
/// settings, and to say what resuming one would do.
pub struct Catalog {
    index: index::Index,
    /// Every session in the index, in no particular order; shared with what
    /// each search finds, so a search holds no borrow of the catalog.
    sessions: Vec<Rc<Listed>>,
    /// What `sessions` are marked hidden by, kept so each snapshot applied
    /// is marked the same way.
    hides: prefs::Hides,
    /// The settings `parse` and `completions` follow.
    settings: Settings,
    /// Where the index and the preferences live.
    data: DataDir,
    providers: Vec<&'static str>,
    /// One per provider, to say how each of its sessions resumes.
    adapters: Vec<Box<dyn Adapter>>,
    /// Where `resume` looks for providers' programs.
    path: OsString,
    malformed_lines: usize,
    searched: Vec<PathBuf>,
    files_read: usize,
    bytes_read: u64,
    /// Every session's key, a new set whenever `sessions` changes which
    /// sessions it holds, so a lookup over the old set is never narrowed
    /// from: a new session may match what it ruled out.
    keys: Arc<HashSet<SessionKey>>,
    /// What `search` narrows from.
    matcher: RefCell<Matcher>,
    /// What runs each `ask`'s search; `None` searches on the caller's thread.
    searcher: Option<Searcher>,
}

/// What one query found in a catalog.
pub struct Found {
    /// The query these sessions were found for.
    pub query: Query,
    /// In `search`'s order.
    pub sessions: Vec<Rc<Listed>>,
    pub left_out: LeftOut,
}

impl Catalog {
    /// Brings the index up to date as `list` does, then reads the hides, the
    /// settings and every session's location from the disk once. Fails only
    /// as `list` does.
    pub fn open(roots: &Roots, data: &DataDir) -> Result<Catalog, DataError> {
        let refreshed = refresh(roots, data)?;
        let snapshot = Snapshot::of(index::Sessions {
            sessions: refreshed.sessions,
            malformed_lines: refreshed.malformed_lines,
        });
        let opening = Opening {
            searched: refreshed.searched,
            files_read: refreshed.files_read,
            bytes_read: refreshed.bytes_read,
        };
        Catalog::with(refreshed.index, snapshot, opening, roots, data)
    }

    /// Every session already in the index, with locations, hides and settings
    /// read now; opens no history file. A missing index is created empty, a
    /// corrupt or old one is recreated. Fails as `open` does.
    pub fn cached(roots: &Roots, data: &DataDir) -> Result<Catalog, DataError> {
        let index = index::Index::open(&data.index())?;
        let snapshot = Snapshot::of(index.sessions(&roots.providers())?);
        let opening = Opening {
            searched: roots
                .history_dirs()
                .into_iter()
                .map(|dir| dir.dir)
                .collect(),
            files_read: 0,
            bytes_read: 0,
        };
        Catalog::with(index, snapshot, opening, roots, data)
    }

    /// The catalog over `index` holding `snapshot`'s sessions, with the hides
    /// and settings read from `data` now.
    fn with(
        index: index::Index,
        snapshot: Snapshot,
        opening: Opening,
        roots: &Roots,
        data: &DataDir,
    ) -> Result<Catalog, DataError> {
        let prefs = prefs::read(&data.prefs())?;
        let mut catalog = Catalog {
            index,
            sessions: Vec::new(),
            hides: prefs.hides,
            settings: prefs.settings,
            data: data.clone(),
            providers: roots.providers(),
            adapters: adapters(roots).into(),
            path: std::env::var_os("PATH").unwrap_or_default(),
            malformed_lines: 0,
            searched: opening.searched,
            files_read: opening.files_read,
            bytes_read: opening.bytes_read,
            keys: Arc::default(),
            matcher: RefCell::default(),
            searcher: None,
        };
        catalog.apply(snapshot);
        Ok(catalog)
    }

    /// Takes `snapshot`'s sessions as this catalog's own, marked hidden by
    /// this catalog's hides; keeps its settings and its index connection.
    pub(crate) fn apply(&mut self, snapshot: Snapshot) {
        self.sessions = snapshot
            .sessions
            .into_iter()
            .map(|listed| {
                let hidden = self.hides.contains(&listed.session);
                Rc::new(Listed { hidden, ..listed })
            })
            .collect();
        self.keys = Arc::new(
            self.sessions
                .iter()
                .map(|listed| SessionKey::of(&listed.session))
                .collect(),
        );
        self.malformed_lines = snapshot.malformed_lines;
    }

    /// `text` read as the query language, with this catalog's providers,
    /// following its settings: temp sessions are left out only while
    /// `hide_temp` is on, and orphaned ones come last only while
    /// `orphans_last` is on. `Query::parse` always follows the defaults.
    pub fn parse(&self, text: &str) -> Query {
        Query::parse(text, &self.providers).following(self.settings)
    }

    /// The sessions matching `query`, with the same rules and order as
    /// `search`. A query that only extends the last one searched, as when
    /// the person types on, costs no more than narrowing what that one
    /// found. Fails only when the index can no longer be read.
    pub fn search(&self, query: &Query) -> Result<Found, DataError> {
        let matched = self
            .matcher
            .borrow_mut()
            .matching(&self.index, query, &self.keys)?;
        Ok(self.found(query.clone(), &matched))
    }

    /// Asks for the sessions matching `query`, as `search` finds them. A
    /// catalog without a worker searches now and gives the results. One
    /// searching in the background or stepped gives `None`, and the results
    /// come through `answer`; an earlier ask whose search hasn't started is
    /// dropped.
    pub(crate) fn ask(&mut self, query: Query) -> Option<Result<Found, DataError>> {
        match &mut self.searcher {
            Some(searcher) => {
                searcher.ask(query, Arc::clone(&self.keys));
                None
            }
            None => Some(self.search(&query)),
        }
    }

    /// The newest results found for the query last asked and not handed
    /// over before; `None` while none has come. Results for any other query
    /// are never handed over. Sessions are listed with the hides as they are
    /// now; a session added since the ask is missing until an ask after it
    /// is answered.
    pub(crate) fn answer(&mut self) -> Option<Result<Found, DataError>> {
        let Matched { query, keys } = self.searcher.as_mut()?.take()?;
        Some(keys.map(|keys| self.found(query, &keys)))
    }

    /// Whether the results for the last ask are still to come.
    pub(crate) fn searching(&self) -> bool {
        self.searcher.as_ref().is_some_and(Searcher::searching)
    }

    /// The sessions among `matched` that `query`'s filters and defaults
    /// keep, in `search`'s order.
    fn found(&self, query: Query, matched: &HashSet<SessionKey>) -> Found {
        let mut sessions: Vec<Rc<Listed>> = self
            .sessions
            .iter()
            .filter(|listed| matched.contains(&SessionKey::of(&listed.session)))
            .cloned()
            .collect();
        let left_out = query.select(&mut sessions);
        Found {
            query,
            sessions,
            left_out,
        }
    }

    /// What the person and the assistant said in `session`, and the tools
    /// the assistant called, in the order recorded: tool output and
    /// subagent transcripts are left out. Empty for a session this catalog
    /// doesn't hold. Fails only when the index can no longer be read.
    pub fn conversation(&self, session: &Session) -> Result<Vec<Message>, DataError> {
        self.index.conversation(&SessionKey::of(session))
    }

    /// The ways to finish the filter at `cursor` in `text`, each with how
    /// many sessions its token lists. After `@`, this is the location list:
    /// the places where the sessions `@` lists ran. Each project comes with
    /// its worktrees after it, then the plain folders. Each group is ordered
    /// most recent first, and missing places come last. A row is kept when a
    /// session that the typed `@` token lists ran there. Missing places are
    /// left out while nothing follows the `@` and `hide_missing` is on.
    /// The whole token the cursor is in or at the end of is what is
    /// finished. Empty unless that token starts with `@`, `#`, `is:`, `!` or
    /// `not:`, so a space never offers any, and empty when the token typed
    /// is already the only way to finish it. Counts and rows follow this
    /// catalog's settings, as `parse` does.
    pub fn completions(&self, text: &str, cursor: usize) -> Vec<Completion> {
        query::completions(text, cursor, &self.providers, &self.sessions, self.settings)
    }

    /// Hides `session`, so that only `is:hidden` finds it, or shows it
    /// again, in this catalog and in `prefs.sqlite`, which is created when
    /// missing. Fails, changing nothing, when the preferences can't be written.
    pub fn set_hidden(&mut self, session: &Session, hidden: bool) -> Result<(), DataError> {
        let choice = match hidden {
            true => Choice::Hide,
            false => Choice::Unhide,
        };
        prefs::record(&self.data.prefs(), std::slice::from_ref(session), choice)?;
        self.hides.set(session, hidden);
        let key = SessionKey::of(session);
        for listed in &mut self.sessions {
            if SessionKey::of(&listed.session) == key {
                *listed = Rc::new(Listed {
                    hidden,
                    ..Listed::clone(listed)
                });
            }
        }
        Ok(())
    }

    /// The settings recorded in prefs.sqlite when this catalog opened, as
    /// changed since; defaults for any not recorded.
    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Where this catalog's index and preferences live.
    pub fn data_dir(&self) -> &DataDir {
        &self.data
    }

    /// Records `settings` in prefs.sqlite, which is created when missing or
    /// upgraded with its hides kept, and makes later `parse` and
    /// `completions` follow them. Fails, changing nothing, when the
    /// preferences can't be written.
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), DataError> {
        prefs::record_settings(&self.data.prefs(), &settings)?;
        self.settings = settings;
        Ok(())
    }

    /// What resuming `session` in `dir` would do, read from the disk, the
    /// provider's metadata and this catalog's `PATH` now. Never fails.
    pub fn resume(&self, session: &Session, dir: ResumeDir) -> ResumePlan {
        let adapter = self
            .adapters
            .iter()
            .find(|adapter| adapter.provider() == session.provider)
            .expect("every catalog session comes from one of its adapters");
        resume::plan(adapter.as_ref(), session, dir, &self.path)
    }

    /// This catalog, finding providers' programs on `path` rather than
    /// the `PATH` this process had when it opened.
    pub fn finding_programs_in(self, path: OsString) -> Catalog {
        Catalog { path, ..self }
    }

    /// This catalog, finding each later `ask`'s sessions on a thread of its
    /// own with its own read-only connection to this catalog's index, so
    /// `ask` returns at once and the results come through `answer`. Returns
    /// at once; dropping the catalog never waits for a search.
    pub fn searching_in_background(self) -> Catalog {
        let searcher = Searcher::start(self.data.index());
        Catalog {
            searcher: Some(searcher),
            ..self
        }
    }

    /// The same, but each search runs only when the returned
    /// `SearchStepper` steps, on the stepping thread.
    pub fn searching_stepped(self) -> (Catalog, SearchStepper) {
        let (searcher, stepper) = Searcher::stepped(self.data.index());
        let catalog = Catalog {
            searcher: Some(searcher),
            ..self
        };
        (catalog, stepper)
    }

    /// Whether `session` is among this catalog's sessions.
    pub(crate) fn holds(&self, session: &Session) -> bool {
        let key = SessionKey::of(session);
        self.sessions
            .iter()
            .any(|listed| SessionKey::of(&listed.session) == key)
    }

    /// Every session in the index, whether or not a query keeps it.
    pub fn indexed(&self) -> usize {
        self.sessions.len()
    }
}

/// Hides every session whose id is `id`, temp, orphaned or already hidden
/// alike, so that only `is:hidden` finds it, and returns them as hidden; an
/// empty list when no session has that id. Records the hide in
/// `prefs.sqlite`, creating it when missing, and never touches a provider's
/// history. Fails as `list` does, or when the preferences can't be written.
pub fn hide(roots: &Roots, data: &DataDir, id: &str) -> Result<Vec<Listed>, DataError> {
    set_hidden(roots, data, id, Choice::Hide)
}

/// Undoes `hide`: every session whose id is `id` is listed again, and is
/// returned as shown; an empty list when no session has that id. Fails as
/// `hide` does.
pub fn unhide(roots: &Roots, data: &DataDir, id: &str) -> Result<Vec<Listed>, DataError> {
    set_hidden(roots, data, id, Choice::Unhide)
}

fn set_hidden(
    roots: &Roots,
    data: &DataDir,
    id: &str,
    choice: Choice,
) -> Result<Vec<Listed>, DataError> {
    // Straight from the index, so temp and hidden sessions are found too.
    let matched: Vec<Session> = refresh(roots, data)?
        .sessions
        .into_iter()
        .filter(|session| session.id == id)
        .collect();
    prefs::record(&data.prefs(), &matched, choice)?;
    let hidden = choice == Choice::Hide;
    Ok(matched
        .into_iter()
        .map(|session| listed(session, hidden))
        .collect())
}

/// `session` with its location read from the disk now.
fn listed(session: Session, hidden: bool) -> Listed {
    Listed {
        location: Location::of(&session.working_dir, &session.recorded_git),
        session,
        hidden,
    }
}

/// The index brought up to date, and every session in it, unfiltered and
/// unordered.
fn refresh(roots: &Roots, data: &DataDir) -> Result<index::Refreshed, DataError> {
    let adapters = adapters(roots);
    let adapters: Vec<&dyn Adapter> = adapters.iter().map(Box::as_ref).collect();
    index::refresh(&adapters, &data.index())
}

/// Deletes the index and builds it again from every history file, then
/// returns what `list` would. Never touches `prefs.sqlite`.
pub fn rebuild(roots: &Roots, data: &DataDir) -> Result<Listing, DataError> {
    index::delete(&data.index())?;
    list(roots, data)
}
