//! The index: Yapkeeper's rebuildable SQLite copy of every session and its
//! searchable text, and finds the sessions whose text contains every word
//! of a query and none of its excluded words. It
//! owns the schema and the refresh rule (which history files are opened, and
//! from which byte), keeps each file's digest as text it never looks inside,
//! and hides how text is found fast: full-text tables, the trigram limit and
//! which rows count as tool-level.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior, params, params_from_iter};
use yapkeeper_model::{
    Adapter, Assembled, FOLD_VERSION, Fact, RecordedGit, ResumeCommand, Role, Session,
    SourceDigest, assemble,
};

use crate::query::{self, Query, Word};
use crate::{Cause, DataError, DataFile, LeftOut, Listed, Listing, Location, SessionKey};

const SCHEMA_VERSION: i64 = 2;

// Source paths come from the filesystem, so they are stored as raw bytes.
// Session paths and text come from JSON, so they are always UTF-8.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sources (
    provider TEXT NOT NULL,
    path BLOB PRIMARY KEY,
    dev INTEGER NOT NULL,
    ino INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    "offset" INTEGER NOT NULL,
    parser_version INTEGER NOT NULL,
    fold_version INTEGER NOT NULL,
    malformed_lines INTEGER NOT NULL,
    digest TEXT NOT NULL,
    thread TEXT
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    source_path BLOB NOT NULL,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_by_source ON messages (source_path);
CREATE TABLE IF NOT EXISTS sessions (
    row_id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    working_dir TEXT NOT NULL,
    other_working_dirs TEXT NOT NULL,
    last_active_ns INTEGER NOT NULL,
    branch TEXT,
    repository_url TEXT,
    recap TEXT,
    linked_prs TEXT NOT NULL,
    subagent_count INTEGER NOT NULL,
    resume_program TEXT NOT NULL,
    resume_args TEXT NOT NULL,
    resume_cwd TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS threads (
    provider TEXT NOT NULL,
    thread TEXT NOT NULL,
    session_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS threads_by_thread ON threads (provider, thread);
-- Folded by query::fold in Rust and matched case-sensitively, so a
-- candidate set is never narrower than what Word::matches accepts.
CREATE VIRTUAL TABLE IF NOT EXISTS message_text USING fts5(
    text,
    content = '',
    contentless_delete = 1,
    tokenize = 'trigram case_sensitive 1'
);
CREATE VIRTUAL TABLE IF NOT EXISTS session_text USING fts5(
    title,
    recap,
    content = '',
    contentless_delete = 1,
    tokenize = 'trigram case_sensitive 1'
);
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    DELETE FROM message_text WHERE rowid = old.id;
END;
CREATE TRIGGER IF NOT EXISTS sessions_ad AFTER DELETE ON sessions BEGIN
    DELETE FROM session_text WHERE rowid = old.row_id;
END;
"#;

/// The search word bound as `?1`, quoted as one FTS5 phrase so that quotes,
/// `OR`, `NEAR`, `*` and `:` in it are matched as text.
const PHRASE: &str = r#"'"' || replace(?1, '"', '""') || '"'"#;

/// Messages that aren't tool-level: what the person and the assistant
/// said in the session's own thread. Tool calls, tool output and every
/// subagent thread are tool-level.
const CONVERSATION: &str =
    "messages.role IN ('user', 'assistant') AND threads.thread = threads.session_id";

/// Brings the index at `path` up to date with every source of `adapters`,
/// opening only files that changed since the last call, and returns its
/// sessions that match `query`'s words, in no particular order, with their
/// locations read from the disk now.
pub(crate) fn refresh(
    adapters: &[&dyn Adapter],
    path: &Path,
    query: &Query,
) -> Result<Listing, DataError> {
    let indexed = open(path).and_then(|mut connection| {
        // Immediate, so two listings at once take turns instead of both
        // reading the same files and one failing to write.
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let listing = refresh_in(&transaction, adapters, query)?;
        transaction.commit()?;
        Ok(listing)
    });
    indexed.map_err(|source| DataError {
        path: path.to_path_buf(),
        file: DataFile::Index,
        source,
    })
}

/// Deletes the index at `path` and its SQLite companion files. Succeeds
/// when there is nothing to delete.
pub(crate) fn delete(path: &Path) -> Result<(), DataError> {
    remove(path).map_err(|error| DataError {
        path: path.to_path_buf(),
        file: DataFile::Index,
        source: error.into(),
    })
}

fn remove(path: &Path) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match fs::remove_file(&file) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// The index at `path`, created when missing. A corrupt index, or one with
/// another schema, holds nothing that can't be read again, so it is
/// deleted and created afresh.
fn open(path: &Path) -> Result<Connection, Cause> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    open_as_is(path).or_else(|error| {
        // A busy index is one another listing is using, not a broken one.
        if is_busy(error.as_ref()) {
            return Err(error);
        }
        remove(path)?;
        open_as_is(path)
    })
}

fn is_busy(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<rusqlite::Error>()
        .and_then(rusqlite::Error::sqlite_error_code)
        .is_some_and(|code| matches!(code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
}

fn open_as_is(path: &Path) -> Result<Connection, Cause> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    match version {
        SCHEMA_VERSION => {}
        // A new database; `IF NOT EXISTS` lets two first listings race safely.
        0 => {
            connection.execute_batch(SCHEMA)?;
            connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        _ => return Err(format!("the index has schema version {version}").into()),
    }
    Ok(connection)
}

/// What the index recorded about a source file when it last read it.
struct Stored {
    stat: Stat,
    /// `None` when the file was read by another parser or fold version, or
    /// its digest won't decode, so it must be read again from the start.
    resume: Option<Progress>,
}

/// Where reading a source file starts, and what the bytes before it folded to.
#[derive(Default)]
struct Progress {
    offset: u64,
    digest: SourceDigest,
    malformed_lines: usize,
}

/// A source file's identity, size and mtime.
#[derive(PartialEq, Eq)]
struct Stat {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i64,
}

fn refresh_in(
    transaction: &Transaction,
    adapters: &[&dyn Adapter],
    query: &Query,
) -> Result<Listing, Cause> {
    let mut files_read = 0;
    let mut bytes_read = 0;
    let mut searched = Vec::new();
    for &adapter in adapters {
        let provider = adapter.provider();
        searched.push(adapter.history_dir());
        let mut stored = stored_sources(transaction, adapter)?;
        let mut changed = false;
        for file in adapter.sources() {
            // Taken before reading, so a file that grows during the read is
            // seen as changed next time.
            let Ok(metadata) = fs::metadata(&file) else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let stat = Stat {
                dev: metadata.dev(),
                ino: metadata.ino(),
                size: metadata.size(),
                mtime_ns: nanos(modified),
            };
            // A file that only grew is read on from its last complete line;
            // anything else is read again from the start.
            let mut progress = match stored.remove(&file) {
                Some(Stored {
                    stat: old,
                    resume: Some(_),
                }) if old == stat => continue,
                Some(Stored {
                    stat: old,
                    resume: Some(progress),
                }) if (old.dev, old.ino) == (stat.dev, stat.ino)
                    && stat.size >= progress.offset =>
                {
                    progress
                }
                _ => {
                    transaction.execute(
                        "DELETE FROM messages WHERE source_path = ?1",
                        [file.as_os_str().as_bytes()],
                    )?;
                    Progress::default()
                }
            };
            let reading = adapter.read(&file, progress.offset);
            files_read += 1;
            bytes_read += stat.size.saturating_sub(progress.offset);
            let digest = &mut progress.digest;
            digest.absorb(&reading.facts, modified);
            store_messages(transaction, &file, &reading.facts)?;
            transaction.execute(
                r#"INSERT OR REPLACE INTO sources (provider, path, dev, ino, size, mtime_ns,
                    "offset", parser_version, fold_version, malformed_lines, digest, thread)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
                params![
                    provider,
                    file.as_os_str().as_bytes(),
                    stat.dev as i64,
                    stat.ino as i64,
                    stat.size as i64,
                    stat.mtime_ns,
                    reading.offset as i64,
                    adapter.parser_version(),
                    FOLD_VERSION,
                    (progress.malformed_lines + reading.malformed_lines) as i64,
                    digest.encode(),
                    digest.thread(),
                ],
            )?;
            changed = true;
        }
        // Whatever is left was indexed before but is no longer a source.
        for gone in stored.keys() {
            let gone = gone.as_os_str().as_bytes();
            transaction.execute("DELETE FROM sources WHERE path = ?1", [gone])?;
            transaction.execute("DELETE FROM messages WHERE source_path = ?1", [gone])?;
            changed = true;
        }
        if changed {
            store_sessions(transaction, adapter)?;
        }
    }
    let malformed_lines: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(malformed_lines), 0) FROM sources",
        [],
        |row| row.get(0),
    )?;
    let mut sessions = Vec::new();
    for &adapter in adapters {
        sessions.extend(stored_sessions(transaction, adapter)?);
    }
    let indexed = sessions.len();
    retain_matching(transaction, query, &mut sessions)?;
    Ok(Listing {
        sessions: sessions
            .into_iter()
            .map(|session| Listed {
                location: Location::of(&session.working_dir, &session.recorded_git),
                session,
                // Hides live in the preferences, which the index never reads.
                hidden: false,
            })
            .collect(),
        indexed,
        malformed_lines: malformed_lines as usize,
        searched,
        files_read,
        bytes_read,
        // Only the query's selection knows what its defaults leave out.
        left_out: LeftOut::default(),
    })
}

/// Every source of `adapter` the index has read, by path.
fn stored_sources(
    transaction: &Transaction,
    adapter: &dyn Adapter,
) -> Result<HashMap<PathBuf, Stored>, Cause> {
    let mut statement = transaction.prepare(
        r#"SELECT path, dev, ino, size, mtime_ns, "offset", malformed_lines, digest,
            parser_version, fold_version
        FROM sources WHERE provider = ?1"#,
    )?;
    let rows = statement.query_map([adapter.provider()], |row| {
        let path: Vec<u8> = row.get(0)?;
        Ok((
            PathBuf::from(OsStr::from_bytes(&path)),
            Stored {
                stat: Stat {
                    dev: row.get::<_, i64>(1)? as u64,
                    ino: row.get::<_, i64>(2)? as u64,
                    size: row.get::<_, i64>(3)? as u64,
                    mtime_ns: row.get(4)?,
                },
                resume: if row.get::<_, u32>(8)? == adapter.parser_version()
                    && row.get::<_, u32>(9)? == FOLD_VERSION
                {
                    let digest: String = row.get(7)?;
                    let offset = row.get::<_, i64>(5)? as u64;
                    let malformed_lines = row.get::<_, i64>(6)? as usize;
                    SourceDigest::decode(&digest).map(|digest| Progress {
                        offset,
                        digest,
                        malformed_lines,
                    })
                } else {
                    None
                },
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Appends `facts`' messages to those already stored for `source`, in order.
fn store_messages(transaction: &Transaction, source: &Path, facts: &[Fact]) -> Result<(), Cause> {
    let source = source.as_os_str().as_bytes();
    let next: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq) + 1, 0) FROM messages WHERE source_path = ?1",
        [source],
        |row| row.get(0),
    )?;
    let mut statement = transaction
        .prepare("INSERT INTO messages (source_path, seq, role, text) VALUES (?1, ?2, ?3, ?4)")?;
    let mut searchable =
        transaction.prepare("INSERT INTO message_text (rowid, text) VALUES (?1, ?2)")?;
    let messages = facts.iter().filter_map(|fact| match fact {
        Fact::Message(message) => Some(message),
        _ => None,
    });
    for (seq, message) in (next..).zip(messages) {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::ToolCall => "tool_call",
            Role::ToolOutput => "tool_output",
        };
        statement.execute(params![source, seq, role, message.text])?;
        searchable.execute(params![
            transaction.last_insert_rowid(),
            query::fold(&message.text)
        ])?;
    }
    Ok(())
}

/// Replaces `adapter`'s sessions and threads with those its stored digests
/// assemble to.
fn store_sessions(transaction: &Transaction, adapter: &dyn Adapter) -> Result<(), Cause> {
    let provider = adapter.provider();
    let mut statement =
        transaction.prepare("SELECT digest FROM sources WHERE provider = ?1 ORDER BY path")?;
    let digests: Vec<SourceDigest> = statement
        .query_map([provider], |row| row.get::<_, String>(0))?
        .filter_map(|digest| SourceDigest::decode(&digest.ok()?))
        .collect();
    let Assembled {
        sessions,
        session_of,
    } = assemble(adapter, &digests);
    transaction.execute("DELETE FROM sessions WHERE provider = ?1", [provider])?;
    transaction.execute("DELETE FROM threads WHERE provider = ?1", [provider])?;
    let mut insert = transaction.prepare(
        "INSERT INTO sessions (provider, id, title, working_dir, other_working_dirs,
            last_active_ns, branch, repository_url, recap, linked_prs, subagent_count,
            resume_program, resume_args, resume_cwd)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?;
    let mut searchable = transaction
        .prepare("INSERT INTO session_text (rowid, title, recap) VALUES (?1, ?2, ?3)")?;
    for session in sessions {
        let other_working_dirs: Vec<String> = session
            .other_working_dirs
            .iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect();
        insert.execute(params![
            provider,
            session.id,
            session.title,
            session.working_dir.to_string_lossy(),
            serde_json::to_string(&other_working_dirs)?,
            nanos(session.last_active),
            session.recorded_git.branch,
            session.recorded_git.repository_url,
            session.recap,
            serde_json::to_string(&session.linked_prs)?,
            session.subagent_count as i64,
            session.resume.program,
            serde_json::to_string(&session.resume.args)?,
            session.resume.cwd.to_string_lossy(),
        ])?;
        searchable.execute(params![
            transaction.last_insert_rowid(),
            query::fold(&session.title),
            session.recap.as_deref().map(query::fold),
        ])?;
    }
    let mut insert = transaction
        .prepare("INSERT INTO threads (provider, thread, session_id) VALUES (?1, ?2, ?3)")?;
    for (thread, session_id) in session_of {
        insert.execute(params![provider, thread, session_id])?;
    }
    Ok(())
}

/// The session a search row names in its first two columns, provider then id.
fn key_from_row(row: &rusqlite::Row) -> rusqlite::Result<SessionKey> {
    Ok(SessionKey {
        provider: row.get(0)?,
        id: row.get(1)?,
    })
}

/// Keeps the `sessions` that match every word of `query` and none of its
/// excluded words, in their order: a session matches a word when its title,
/// its recap or one of its messages in scope does.
fn retain_matching(
    transaction: &Transaction,
    query: &Query,
    sessions: &mut Vec<Session>,
) -> Result<(), Cause> {
    let mut words: Vec<_> = query.words().iter().collect();
    // A longer word tends to be rarer, so fewer sessions are left for the rest.
    words.sort_by_key(|word| Reverse(word.key().chars().count()));
    let scope = if query.tools() { "true" } else { CONVERSATION };
    for word in words {
        let matched = sessions_matching(transaction, word, scope, sessions)?;
        sessions.retain(|session| matched.contains(&SessionKey::of(session)));
    }
    for word in query.excluded_words() {
        let matched = sessions_matching(transaction, word, scope, sessions)?;
        sessions.retain(|session| !matched.contains(&SessionKey::of(session)));
    }
    Ok(())
}

/// Which of `sessions` match `word` in their title, recap or a message
/// that `scope`, an SQL condition on `messages` and `threads`, admits.
fn sessions_matching(
    transaction: &Transaction,
    word: &Word,
    scope: &str,
    sessions: &[Session],
) -> Result<HashSet<SessionKey>, Cause> {
    let mut unmatched: HashSet<SessionKey> = sessions.iter().map(SessionKey::of).collect();
    let mut matched = HashSet::new();
    // Trigram rows hold three-character sequences, so a shorter word
    // can't be looked up, and every row in scope is read instead.
    // Bound as `?1` only in the SQL that looks it up.
    let lookup = (word.key().chars().count() >= 3).then_some(word.key());
    let (session_rows, message_rows, match_clause) = match lookup {
        Some(_) => (
            format!(
                "session_text JOIN sessions ON sessions.row_id = session_text.rowid
                WHERE session_text MATCH {PHRASE}"
            ),
            "message_text JOIN messages ON messages.id = message_text.rowid",
            format!("AND message_text MATCH {PHRASE}"),
        ),
        None => ("sessions".to_string(), "messages", String::new()),
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT sessions.provider, sessions.id, sessions.title, sessions.recap
        FROM {session_rows}"
    ))?;
    let mut rows = statement.query(params_from_iter(lookup))?;
    while let Some(row) = rows.next()? {
        let key = key_from_row(row)?;
        let title: String = row.get(2)?;
        let recap: Option<String> = row.get(3)?;
        if unmatched.contains(&key)
            && (word.matches(&title) || recap.is_some_and(|recap| word.matches(&recap)))
        {
            unmatched.remove(&key);
            matched.insert(key);
        }
    }
    let mut statement = transaction.prepare(&format!(
        "SELECT threads.provider, threads.session_id, messages.text
        FROM {message_rows}
        JOIN sources ON sources.path = messages.source_path
        JOIN threads ON threads.provider = sources.provider
            AND threads.thread = sources.thread
        WHERE {scope} {match_clause}"
    ))?;
    let mut rows = statement.query(params_from_iter(lookup))?;
    while let Some(row) = rows.next()? {
        let key = key_from_row(row)?;
        if unmatched.contains(&key) && word.matches(&row.get::<_, String>(2)?) {
            unmatched.remove(&key);
            matched.insert(key);
        }
    }
    Ok(matched)
}

/// `adapter`'s sessions as the index stores them.
fn stored_sessions(
    transaction: &Transaction,
    adapter: &dyn Adapter,
) -> Result<Vec<Session>, Cause> {
    let mut statement = transaction.prepare(
        "SELECT id, title, working_dir, other_working_dirs, last_active_ns, branch,
            repository_url, recap, linked_prs, subagent_count, resume_program, resume_args,
            resume_cwd
        FROM sessions WHERE provider = ?1",
    )?;
    let mut rows = statement.query([adapter.provider()])?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next()? {
        let other_working_dirs: Vec<String> = serde_json::from_str(&row.get::<_, String>(3)?)?;
        sessions.push(Session {
            provider: adapter.provider(),
            id: row.get(0)?,
            title: row.get(1)?,
            working_dir: PathBuf::from(row.get::<_, String>(2)?),
            other_working_dirs: other_working_dirs.into_iter().map(PathBuf::from).collect(),
            last_active: time(row.get(4)?),
            recorded_git: RecordedGit {
                branch: row.get(5)?,
                repository_url: row.get(6)?,
            },
            recap: row.get(7)?,
            linked_prs: serde_json::from_str(&row.get::<_, String>(8)?)?,
            subagent_count: row.get::<_, i64>(9)? as usize,
            resume: ResumeCommand {
                program: row.get(10)?,
                args: serde_json::from_str(&row.get::<_, String>(11)?)?,
                cwd: PathBuf::from(row.get::<_, String>(12)?),
            },
        });
    }
    Ok(sessions)
}

/// Nanoseconds since the Unix epoch; earlier times are stored as the epoch.
fn nanos(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_nanos()).unwrap_or(i64::MAX)
    })
}

fn time(nanos: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(u64::try_from(nanos).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use yapkeeper_model::{Message, Reading, ThreadKind};

    use super::*;

    /// Gives each file one session whose prompt names the parser version
    /// that read it.
    struct Stub {
        dir: PathBuf,
        version: Cell<u32>,
    }

    impl Adapter for Stub {
        fn provider(&self) -> &'static str {
            "stub"
        }

        fn parser_version(&self) -> u32 {
            self.version.get()
        }

        fn history_dir(&self) -> PathBuf {
            self.dir.clone()
        }

        fn sources(&self) -> Vec<PathBuf> {
            let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            files.sort();
            files
        }

        fn read(&self, source: &Path, from: u64) -> Reading {
            let id = source.file_stem().unwrap().to_string_lossy().into_owned();
            let prompt = format!("Read by parser version {}", self.version.get());
            Reading {
                facts: vec![
                    Fact::Identity {
                        id: Some(id),
                        kind: ThreadKind::Session,
                    },
                    Fact::WorkingDir(PathBuf::from("/synthetic/stub")),
                    Fact::Message(Message::new(Role::User, &prompt).unwrap()),
                ],
                offset: from.max(fs::metadata(source).unwrap().len()),
                malformed_lines: 0,
            }
        }

        fn resume(&self, id: &str, working_dir: &Path) -> ResumeCommand {
            ResumeCommand {
                program: "stub".to_string(),
                args: vec![id.to_string()],
                cwd: working_dir.to_path_buf(),
            }
        }
    }

    #[test]
    fn a_new_parser_version_reads_every_file_of_its_provider_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("history");
        fs::create_dir(&dir).unwrap();
        for name in ["one", "two"] {
            fs::write(dir.join(format!("{name}.jsonl")), "{}\n").unwrap();
        }
        let stub = Stub {
            dir,
            version: Cell::new(1),
        };
        let index = tmp.path().join("index.sqlite");
        refresh(&[&stub], &index, &Query::default()).unwrap();
        assert_eq!(
            refresh(&[&stub], &index, &Query::default())
                .unwrap()
                .files_read,
            0
        );

        stub.version.set(2);
        let listing = refresh(&[&stub], &index, &Query::default()).unwrap();
        assert_eq!(listing.files_read, 2);
        let titles: Vec<&str> = listing
            .sessions
            .iter()
            .map(|listed| listed.session.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["Read by parser version 2", "Read by parser version 2"]
        );
    }
}
