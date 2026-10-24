//! The preferences: the user's own choices, hides and settings, in a SQLite
//! file kept apart from the index so rebuilding it never loses them. It
//! owns that file's schema, its upgrade from older versions, how a session
//! is named in it and how each setting is spelt.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use yapkeeper_model::Session;

use crate::{Accent, BUSY_TIMEOUT, Cause, DataError, DataFile, IconSet, SessionKey, Settings};

const SCHEMA_VERSION: i64 = 2;

/// Version 1 had only the hides; it has no `setting` table.
const HIDES_ONLY: i64 = 1;

// Keyed by provider as well as id, so two providers can never share a hide.
// Every statement creates only what is missing, so it also upgrades a file
// of an older version, keeping what it holds.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS hidden (
    provider TEXT NOT NULL,
    id TEXT NOT NULL,
    PRIMARY KEY (provider, id)
);
CREATE TABLE IF NOT EXISTS setting (
    name TEXT PRIMARY KEY,
    value TEXT NOT NULL
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

    /// Records `session` as hidden, or as shown, in these hides only.
    pub(crate) fn set(&mut self, session: &Session, hidden: bool) {
        let key = SessionKey::of(session);
        match hidden {
            true => self.sessions.insert(key),
            false => self.sessions.remove(&key),
        };
    }
}

/// Everything the preferences hold.
pub(crate) struct Prefs {
    pub(crate) hides: Hides,
    pub(crate) settings: Settings,
}

/// The hides and settings recorded at `path`. A missing file holds no hides
/// and the default settings, and is not created; so does any setting it
/// doesn't record, or records with a value this version doesn't know.
pub(crate) fn read(path: &Path) -> Result<Prefs, DataError> {
    read_file(path).map_err(|source| failure(path, source))
}

fn read_file(path: &Path) -> Result<Prefs, Cause> {
    if !path.exists() {
        return Ok(Prefs {
            hides: Hides {
                sessions: HashSet::new(),
            },
            settings: Settings::default(),
        });
    }
    // Read-only, so listing never changes the file.
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    let has_settings = match version(&connection)? {
        HIDES_ONLY => false,
        _ => {
            check_version(&connection)?;
            true
        }
    };
    let mut statement = connection.prepare("SELECT provider, id FROM hidden")?;
    let rows = statement.query_map([], |row| {
        Ok(SessionKey {
            provider: row.get(0)?,
            id: row.get(1)?,
        })
    })?;
    let sessions = rows.collect::<Result<_, _>>()?;
    let mut settings = Settings::default();
    if has_settings {
        let mut statement = connection.prepare("SELECT name, value FROM setting")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            let (name, value): (String, String) = row?;
            read_setting(&mut settings, &name, &value);
        }
    }
    Ok(Prefs {
        hides: Hides { sessions },
        settings,
    })
}

/// How `on` is spelt in the file.
fn toggle_spelling(on: bool) -> &'static str {
    match on {
        true => "on",
        false => "off",
    }
}

/// How `accent` is spelt in the file.
fn accent_spelling(accent: Accent) -> &'static str {
    match accent {
        Accent::Sage => "sage",
        Accent::Cyan => "cyan",
        Accent::Amber => "amber",
        Accent::Violet => "violet",
        Accent::Sky => "sky",
        Accent::Mono => "mono",
    }
}

/// How `set` is spelt in the file.
fn icon_set_spelling(set: IconSet) -> &'static str {
    match set {
        IconSet::NerdFont => "nerd-font",
        IconSet::Unicode => "unicode",
        IconSet::TextTags => "text",
        IconSet::None => "none",
    }
}

/// Each setting's row as the file spells it, name then value.
fn setting_rows(settings: &Settings) -> [(&'static str, &'static str); 5] {
    [
        ("hide_temp", toggle_spelling(settings.hide_temp)),
        ("hide_missing", toggle_spelling(settings.hide_missing)),
        ("orphans_last", toggle_spelling(settings.orphans_last)),
        ("accent", accent_spelling(settings.accent)),
        ("icons", icon_set_spelling(settings.icons)),
    ]
}

/// Sets the setting `name` to `value` as the file spells them; a name or a
/// value this version doesn't know leaves `settings` as they are, so a newer
/// version's choices fall back to the defaults rather than failing.
fn read_setting(settings: &mut Settings, name: &str, value: &str) {
    /// The one of `all` that `spelling` spells as `value`, else `current`.
    fn named<T: Copy>(all: &[T], spelling: fn(T) -> &'static str, value: &str, current: T) -> T {
        all.iter()
            .copied()
            .find(|known| spelling(*known) == value)
            .unwrap_or(current)
    }
    const TOGGLES: [bool; 2] = [true, false];
    match name {
        "hide_temp" => {
            settings.hide_temp = named(&TOGGLES, toggle_spelling, value, settings.hide_temp)
        }
        "hide_missing" => {
            settings.hide_missing = named(&TOGGLES, toggle_spelling, value, settings.hide_missing)
        }
        "orphans_last" => {
            settings.orphans_last = named(&TOGGLES, toggle_spelling, value, settings.orphans_last)
        }
        "accent" => settings.accent = named(&Accent::ALL, accent_spelling, value, settings.accent),
        "icons" => settings.icons = named(&IconSet::ALL, icon_set_spelling, value, settings.icons),
        _ => {}
    }
}

/// Records every one of `settings` at `path`, creating the file when
/// missing or upgrading it with its hides kept.
pub(crate) fn record_settings(path: &Path, settings: &Settings) -> Result<(), DataError> {
    write_settings(path, settings).map_err(|source| failure(path, source))
}

fn write_settings(path: &Path, settings: &Settings) -> Result<(), Cause> {
    let mut connection = open_for_writing(path)?;
    let transaction = connection.transaction()?;
    for (name, value) in setting_rows(settings) {
        transaction.execute(
            "INSERT OR REPLACE INTO setting (name, value) VALUES (?1, ?2)",
            params![name, value],
        )?;
    }
    transaction.commit()?;
    Ok(())
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
    let mut connection = open_for_writing(path)?;
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

/// The file at `path`, created with its directory when missing and upgraded
/// to this version when older, keeping what it holds.
fn open_for_writing(path: &Path) -> Result<Connection, Cause> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    if matches!(version(&connection)?, 0 | HIDES_ONLY) {
        connection.execute_batch(SCHEMA)?;
        connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    }
    check_version(&connection)?;
    Ok(connection)
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
