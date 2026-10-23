//! The preferences: the user's own choices, in a SQLite file kept apart from
//! the index so rebuilding it never loses them. It owns that file's schema
//! and how a session is named in it.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use yapkeeper_model::Session;

use crate::{BUSY_TIMEOUT, Cause, DataError, DataFile, SessionKey};

const SCHEMA_VERSION: i64 = 1;

// Keyed by provider as well as id, so two providers can never share a hide.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS hidden (
    provider TEXT NOT NULL,
    id TEXT NOT NULL,
    PRIMARY KEY (provider, id)
);
"#;

/// What to record for a session.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    Hide,
    Unhide,
}

/// The sessions the user has hidden.
pub(crate) struct Hides {
    sessions: HashSet<SessionKey>,
}

impl Hides {
    pub(crate) fn contains(&self, session: &Session) -> bool {
        // Built once per call, as a set can't be probed with borrowed parts.
        self.sessions.contains(&SessionKey::of(session))
    }
}

/// The hides recorded at `path`. A missing file holds none and is not created.
pub(crate) fn hides(path: &Path) -> Result<Hides, DataError> {
    read(path)
        .map(|sessions| Hides { sessions })
        .map_err(|source| failure(path, source))
}

fn read(path: &Path) -> Result<HashSet<SessionKey>, Cause> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    // Read-only, so listing never changes the file.
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    check_version(&connection)?;
    let mut statement = connection.prepare("SELECT provider, id FROM hidden")?;
    let rows = statement.query_map([], |row| {
        Ok(SessionKey {
            provider: row.get(0)?,
            id: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Records `choice` for each of `sessions`, creating the file at `path` when
/// missing. Hiding a hidden session or unhiding a shown one changes nothing.
pub(crate) fn record(path: &Path, sessions: &[Session], choice: Choice) -> Result<(), DataError> {
    write(path, sessions, choice).map_err(|source| failure(path, source))
}

fn failure(path: &Path, source: Cause) -> DataError {
    DataError {
        path: path.to_path_buf(),
        file: DataFile::Preferences,
        source,
    }
}

fn write(path: &Path, sessions: &[Session], choice: Choice) -> Result<(), Cause> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    if version(&connection)? == 0 {
        connection.execute_batch(SCHEMA)?;
        connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    }
    check_version(&connection)?;
    let transaction = connection.transaction()?;
    let statement = match choice {
        Choice::Hide => "INSERT OR IGNORE INTO hidden (provider, id) VALUES (?1, ?2)",
        Choice::Unhide => "DELETE FROM hidden WHERE provider = ?1 AND id = ?2",
    };
    for session in sessions {
        transaction.execute(statement, params![session.provider, session.id])?;
    }
    transaction.commit()?;
    Ok(())
}

/// Unlike the index, preferences can't be read again from anywhere, so a
/// file of another version is an error rather than something to replace.
fn check_version(connection: &Connection) -> Result<(), Cause> {
    match version(connection)? {
        SCHEMA_VERSION => Ok(()),
        version => Err(format!("the preferences have schema version {version}").into()),
    }
}

/// The file's schema version; 0 for a file Yapkeeper has not set up.
fn version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("PRAGMA user_version", [], |row| row.get(0))
}
