//! Sessions from every provider, merged and ordered, each with where it ran.
//! Each provider's format stays inside its adapter; this crate knows where
//! each one is rooted and where Yapkeeper keeps its own files, keeps the
//! index of every session up to date, finds sessions by what was said and
//! by the query language's filters, reads each session's location from
//! the disk when it is listed, and keeps the user's hides apart from the
//! index.

#![forbid(unsafe_code)]

mod index;
mod location;
mod prefs;
mod query;

use prefs::Choice;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use yapkeeper_claude::ClaudeCode;
use yapkeeper_codex::Codex;

pub use location::*;
pub use query::{Query, State, Token, TokenKind};
pub use yapkeeper_model::*;

/// Where each provider keeps its history.
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
}

impl Roots {
    /// The roots this process's environment names. Fails only when no home
    /// directory can be found for a provider that needs one.
    pub fn from_env() -> Result<Roots, MissingHome> {
        Ok(Roots {
            claude: yapkeeper_claude::config_dir()?,
            codex: yapkeeper_codex::home_dir()?,
        })
    }

    /// The provider name of each adapter `search` reads, in search order,
    /// such as `["claude", "codex"]`. Opens nothing.
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
fn adapters(roots: &Roots) -> [Box<dyn Adapter>; 2] {
    [
        Box::new(ClaudeCode::at(roots.claude.clone())),
        Box::new(Codex::at(roots.codex.clone())),
    ]
}

/// Where Yapkeeper keeps its own files: `$XDG_DATA_HOME/yapkeeper` when that
/// is set and absolute, otherwise `$HOME/.local/share/yapkeeper`. Creates nothing.
pub struct DataDir {
    dir: PathBuf,
}

impl DataDir {
    /// Fails with `MissingHome { variable: "XDG_DATA_HOME" }` when neither names a directory.
    pub fn from_env() -> Result<DataDir, MissingHome> {
        // The XDG spec says a relative path is invalid and must be ignored.
        let base = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            Some(dir) if dir.is_absolute() => dir,
            _ => match std::env::var_os("HOME") {
                Some(home) if !home.is_empty() => PathBuf::from(home).join(".local/share"),
                _ => {
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

    /// `<dir>/prefs.sqlite`, the user's hides; never touched by the index.
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
    /// Which defaults left out a session the query otherwise matched.
    pub left_out: LeftOut,
}

/// The defaults that left sessions out of a listing: temp sessions unless
/// the query has `is:temp`, and hidden ones unless it has `is:hidden`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeftOut {
    pub temp: bool,
    pub hidden: bool,
}

/// A session with its location and whether it is hidden, as of this listing.
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

/// Why one of Yapkeeper's own files failed.
type Cause = Box<dyn Error + Send + Sync>;

/// A session as the index and the preferences name it: by provider as well
/// as id, so two providers can never share one.
#[derive(PartialEq, Eq, Hash)]
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
            DataFile::Index => "write the index",
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
    let mut listing = refresh(roots, data, query)?;
    let hides = prefs::hides(&data.prefs())?;
    for listed in &mut listing.sessions {
        listed.hidden = hides.contains(&listed.session);
    }
    listing.left_out = query.select(&mut listing.sessions);
    Ok(listing)
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
    let listing = refresh(roots, data, &Query::default())?;
    let mut sessions: Vec<Listed> = listing
        .sessions
        .into_iter()
        .filter(|listed| listed.session.id == id)
        .collect();
    let matched: Vec<&Session> = sessions.iter().map(|listed| &listed.session).collect();
    prefs::record(&data.prefs(), &matched, choice)?;
    for listed in &mut sessions {
        listed.hidden = choice == Choice::Hide;
    }
    Ok(sessions)
}

/// The index brought up to date, and its sessions with `query`'s words,
/// unfiltered, unordered and none hidden yet.
fn refresh(roots: &Roots, data: &DataDir, query: &Query) -> Result<Listing, DataError> {
    let adapters = adapters(roots);
    let adapters: Vec<&dyn Adapter> = adapters.iter().map(Box::as_ref).collect();
    index::refresh(&adapters, &data.index(), query)
}

/// Deletes the index and builds it again from every history file, then
/// returns what `list` would. Never touches `prefs.sqlite`.
pub fn rebuild(roots: &Roots, data: &DataDir) -> Result<Listing, DataError> {
    index::delete(&data.index())?;
    list(roots, data)
}
