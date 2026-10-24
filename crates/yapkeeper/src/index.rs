//! The index: Yapkeeper's rebuildable SQLite copy of every session and its
//! searchable text. It finds the sessions whose text contains every word of
//! a query and none of its excluded words. It owns the schema and the
//! refresh rule (which history files are opened, and from which byte),
//! applied in small committed batches, safe beside another writer,
//! keeps each file's digest as text it never looks inside, and hides how
//! text is found fast: full-text tables, the trigram limit, the stored folds
//! that words too short for it are matched in, and which rows count as
//! tool-level.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension, ToSql, Transaction, TransactionBehavior, params,
};
use yapkeeper_model::{
    Adapter, Assembled, FOLD_VERSION, Fact, Message, RecordedGit, ResumeCommand, Role, Session,
    SourceDigest, assemble,
};

use crate::query::{self, Query, Word};
use crate::{BUSY_TIMEOUT, Cause, DataError, DataFile, SessionKey};

const SCHEMA_VERSION: i64 = 4;

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
CREATE INDEX IF NOT EXISTS sources_by_thread ON sources (provider, thread);
CREATE TABLE IF NOT EXISTS sessions (
    row_id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    title_fold TEXT NOT NULL,
    working_dir TEXT NOT NULL,
    other_working_dirs TEXT NOT NULL,
    last_active_ns INTEGER NOT NULL,
    branch TEXT,
    repository_url TEXT,
    recap TEXT,
    recap_fold TEXT,
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
CREATE INDEX IF NOT EXISTS threads_by_session ON threads (provider, session_id);
-- Each read of a source also stores its messages, each ended by a newline,
-- which no search word can span, as up to three documents: what the
-- person and the assistant said ('said', folded by query::fold, and
-- 'said as typed'), and the tool-level text ('tool', folded). Words
-- too short for the trigram tables are matched in these with instr.
CREATE TABLE IF NOT EXISTS documents (
    id INTEGER PRIMARY KEY,
    source_path BLOB NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS documents_by_source ON documents (source_path);
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

/// The search word bound as `?2`, quoted as one FTS5 phrase so that quotes,
/// `OR`, `NEAR`, `*` and `:` in it are matched as text.
const PHRASE: &str = r#"'"' || replace(?2, '"', '""') || '"'"#;

/// What the person and the assistant said, leaving out tool calls and
/// tool output. In the session's own thread, these messages are the ones
/// that aren't tool-level. Keep in step with `said`.
const SAID: &str = "messages.role IN ('user', 'assistant')";

/// Whether a message of `role` is one `SAID` admits. Keep in step with `SAID`.
fn said(role: Role) -> bool {
    matches!(role, Role::User | Role::Assistant)
}

/// Threads that are the session's own, leaving out every subagent's.
const OWN_THREAD: &str = "threads.thread = threads.session_id";

/// The sessions still in play, bound as `?1`: a JSON array of
/// `[provider, id]` pairs.
const IN_PLAY: &str = "SELECT value ->> 0, value ->> 1 FROM json_each(?1)";

/// The index, open; each read sees the last batch any writer committed.
pub(crate) struct Index {
    connection: Connection,
    path: PathBuf,
}

/// Every session in the index as of one read, in no particular order.
pub(crate) struct Sessions {
    pub(crate) sessions: Vec<Session>,
    /// Malformed complete lines in every indexed file.
    pub(crate) malformed_lines: usize,
}

impl Index {
    /// The index at `path`, created when missing, recreated when corrupt or of
    /// another schema; refreshes nothing.
    pub(crate) fn open(path: &Path) -> Result<Index, DataError> {
        match open(path) {
            Ok(connection) => Ok(Index {
                connection,
                path: path.to_path_buf(),
            }),
            Err(source) => Err(DataError {
                path: path.to_path_buf(),
                file: DataFile::Index,
                source,
            }),
        }
    }

    /// Every session of `providers` and the malformed-line total, read in
    /// one read transaction.
    pub(crate) fn sessions(&self, providers: &[&'static str]) -> Result<Sessions, DataError> {
        sessions(&self.connection, providers).map_err(|source| self.error(source))
    }

    /// The `candidates` whose title, recap or a message in scope contains
    /// every word of `query`, and none of its excluded words. Fails only
    /// when the index can no longer be read.
    pub(crate) fn matching(
        &self,
        query: &Query,
        candidates: HashSet<SessionKey>,
    ) -> Result<HashSet<SessionKey>, DataError> {
        narrow(&self.connection, query, candidates).map_err(|source| self.error(source))
    }

    /// What the person and the assistant said in `session`'s own thread,
    /// and the tools the assistant called there, in the order recorded;
    /// empty for a session the index doesn't hold.
    /// Fails only when the index can no longer be read.
    pub(crate) fn conversation(&self, session: &SessionKey) -> Result<Vec<Message>, DataError> {
        conversation(&self.connection, session).map_err(|source| self.error(source))
    }

    fn error(&self, source: Cause) -> DataError {
        DataError {
            path: self.path.clone(),
            file: DataFile::Index,
            source,
        }
    }
}

/// `Index::sessions` before its error names the index.
fn sessions(connection: &Connection, providers: &[&'static str]) -> Result<Sessions, Cause> {
    // Both reads see the same commit, so the total matches the sessions.
    let transaction = connection.unchecked_transaction()?;
    let malformed_lines: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(malformed_lines), 0) FROM sources",
        [],
        |row| row.get(0),
    )?;
    let mut sessions = Vec::new();
    for &provider in providers {
        sessions.extend(stored_sessions(&transaction, provider)?);
    }
    Ok(Sessions {
        sessions,
        malformed_lines: malformed_lines as usize,
    })
}

/// `Index::conversation` before its error names the index.
fn conversation(connection: &Connection, session: &SessionKey) -> Result<Vec<Message>, Cause> {
    let mut statement = connection.prepare(&format!(
        "SELECT messages.role, messages.text
        FROM messages
        JOIN sources ON sources.path = messages.source_path
        JOIN threads ON threads.provider = sources.provider
            AND threads.thread = sources.thread
        WHERE threads.provider = ?1 AND threads.session_id = ?2 AND {OWN_THREAD}
            AND messages.role != 'tool_output'
        ORDER BY sources.path, messages.seq"
    ))?;
    let messages = statement
        .query_map(params![session.provider, session.id], |row| {
            let stored = row.get::<_, String>(0)?;
            let role = [Role::User, Role::Assistant, Role::ToolCall]
                .into_iter()
                .find(|role| role_name(*role) == stored)
                .unwrap_or(Role::Assistant);
            Ok(Message {
                role,
                text: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(messages)
}

/// `Index::matching` before its error names the index.
fn narrow(
    connection: &Connection,
    query: &Query,
    mut candidates: HashSet<SessionKey>,
) -> Result<HashSet<SessionKey>, Cause> {
    let mut words: Vec<_> = query.words().iter().collect();
    // A longer word tends to be rarer, so fewer sessions are left for the rest.
    words.sort_by_key(|word| Reverse(word.key().chars().count()));
    for word in words {
        let matched = sessions_matching(connection, word, query.tools(), &candidates)?;
        candidates.retain(|key| matched.contains(key));
    }
    for word in query.excluded_words() {
        let matched = sessions_matching(connection, word, query.tools(), &candidates)?;
        candidates.retain(|key| !matched.contains(key));
    }
    Ok(candidates)
}

/// What a refresh of the index found: every session in it, in no particular
/// order, and what the refresh read.
pub(crate) struct Refreshed {
    pub(crate) index: Index,
    pub(crate) sessions: Vec<Session>,
    /// Malformed complete lines in every indexed file.
    pub(crate) malformed_lines: usize,
    /// Each adapter's history directory, in adapter order.
    pub(crate) searched: Vec<PathBuf>,
    /// History files opened during this call.
    pub(crate) files_read: usize,
    /// History bytes read during this call.
    pub(crate) bytes_read: u64,
}

/// Brings the index at `path` up to date with every source of `adapters`,
/// opening only files that changed since the last call, and returns every
/// session in it, with the index kept open for word lookups. Fails only
/// when the index can't be created or written.
pub(crate) fn refresh(adapters: &[&dyn Adapter], path: &Path) -> Result<Refreshed, DataError> {
    let mut pass = Pass::new(adapters, path)?;
    let never = AtomicBool::new(false);
    while pass.next_batch(adapters, &never)? {}
    let Sessions {
        sessions,
        malformed_lines,
    } = pass.sessions()?;
    Ok(Refreshed {
        index: pass.index,
        sessions,
        malformed_lines,
        searched: adapters
            .iter()
            .map(|adapter| adapter.history_dir())
            .collect(),
        files_read: pass.files_read,
        bytes_read: pass.bytes_read,
    })
}

/// The most files one batch reads, and the new bytes after which it takes
/// no more, so each batch holds the write lock briefly.
const BATCH_FILES: usize = 32;
const BATCH_BYTES: u64 = 16 * 1024 * 1024;

/// One refresh, split into batches, each committed on its own so readers
/// see it at once and another writer can take its turn between them.
pub(crate) struct Pass {
    index: Index,
    /// The providers of the pass's adapters, whose sessions `sessions` reads.
    providers: Vec<&'static str>,
    /// The files to read, oldest mtime first, so the newest is read first.
    planned: Vec<Planned>,
    /// Indexed files that are no longer sources; the first batch deletes
    /// what they recorded.
    gone: Vec<Gone>,
    /// History files opened by the batches committed so far.
    files_read: usize,
    /// History bytes read by the batches committed so far.
    bytes_read: u64,
}

/// A file a pass reads, as planned.
struct Planned {
    /// Which of the pass's adapters it is a source of.
    adapter: usize,
    file: PathBuf,
    /// Its mtime as planned, which orders the plan.
    mtime_ns: i64,
}

/// An indexed file that is no longer a source.
struct Gone {
    /// Which of the pass's adapters it was a source of.
    adapter: usize,
    file: PathBuf,
}

impl Pass {
    /// Opens the index as `Index::open` does, lists and stats every source
    /// of `adapters`, and plans the files to read (newest mtime first) and
    /// the ones gone. Takes no write lock.
    pub(crate) fn new(adapters: &[&dyn Adapter], path: &Path) -> Result<Pass, DataError> {
        let mut pass = Pass {
            index: Index::open(path)?,
            providers: adapters.iter().map(|adapter| adapter.provider()).collect(),
            planned: Vec::new(),
            gone: Vec::new(),
            files_read: 0,
            bytes_read: 0,
        };
        pass.plan(adapters)
            .map_err(|source| pass.index.error(source))?;
        Ok(pass)
    }

    fn plan(&mut self, adapters: &[&dyn Adapter]) -> Result<(), Cause> {
        for (n, &adapter) in adapters.iter().enumerate() {
            let mut stored = stored_sources(&self.index.connection, adapter)?;
            for file in adapter.sources() {
                let Some(stat) = stat(&file) else {
                    continue;
                };
                if progress(stored.remove(&file), &stat).is_some() {
                    self.planned.push(Planned {
                        adapter: n,
                        file,
                        mtime_ns: stat.mtime_ns,
                    });
                }
            }
            // Whatever is left was indexed before but is no longer a source.
            self.gone
                .extend(stored.into_keys().map(|file| Gone { adapter: n, file }));
        }
        self.planned.sort_by_key(|planned| planned.mtime_ns);
        Ok(())
    }

    /// Commits the next batch: up to `BATCH_FILES` files or `BATCH_BYTES` new bytes, read
    /// by `adapters`, the ones given to `new`. True when it committed one.
    /// False, committing nothing, once nothing is left or `stop` is set. A
    /// busy index fails it as it fails `refresh`.
    pub(crate) fn next_batch(
        &mut self,
        adapters: &[&dyn Adapter],
        stop: &AtomicBool,
    ) -> Result<bool, DataError> {
        if stop.load(Ordering::Relaxed) || (self.planned.is_empty() && self.gone.is_empty()) {
            return Ok(false);
        }
        self.batch(adapters, stop)
            .map_err(|source| self.index.error(source))
    }

    fn batch(&mut self, adapters: &[&dyn Adapter], stop: &AtomicBool) -> Result<bool, Cause> {
        // Immediate, so two writers take turns instead of both reading the
        // same files and one failing to write.
        let transaction = self
            .index
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changed = vec![false; adapters.len()];
        for gone in &self.gone {
            let gone_path = gone.file.as_os_str().as_bytes();
            transaction.execute("DELETE FROM sources WHERE path = ?1", [gone_path])?;
            forget_messages(&transaction, gone_path)?;
            changed[gone.adapter] = true;
        }
        let mut files_read = 0;
        let mut bytes_read = 0;
        while files_read < BATCH_FILES && bytes_read < BATCH_BYTES {
            // Dropping the transaction rolls back what this batch wrote.
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let Some(planned) = self.planned.pop() else {
                break;
            };
            // Decided again inside the lock: another writer may have read
            // the file since this pass planned it, and it may have changed.
            let adapter = adapters[planned.adapter];
            let Some(stat) = stat(&planned.file) else {
                continue;
            };
            let stored = stored_source(&transaction, adapter, &planned.file)?;
            let Some(progress) = progress(stored, &stat) else {
                continue;
            };
            bytes_read += read_source(&transaction, adapter, &planned.file, &stat, progress)?;
            files_read += 1;
            changed[planned.adapter] = true;
        }
        for (&adapter, changed) in adapters.iter().zip(changed) {
            if changed {
                store_sessions(&transaction, adapter)?;
            }
        }
        transaction.commit()?;
        self.gone.clear();
        self.files_read += files_read;
        self.bytes_read += bytes_read;
        Ok(true)
    }

    /// Every session of the pass's providers in the index, as of its last
    /// commit or another writer's since.
    pub(crate) fn sessions(&self) -> Result<Sessions, DataError> {
        self.index.sessions(&self.providers)
    }
}

/// Reads `file`, a source of `adapter` whose stat was `stat` before reading,
/// on from `progress`, and stores what it adds. Gives the bytes read.
fn read_source(
    transaction: &Transaction,
    adapter: &dyn Adapter,
    file: &Path,
    stat: &Stat,
    mut progress: Progress,
) -> Result<u64, Cause> {
    // Read from the start, so whatever was stored for it goes.
    if progress.offset == 0 {
        forget_messages(transaction, file.as_os_str().as_bytes())?;
    }
    let reading = adapter.read(file, progress.offset);
    let digest = &mut progress.digest;
    digest.absorb(&reading.facts, time(stat.mtime_ns));
    store_messages(transaction, file, &reading.facts)?;
    transaction.execute(
        r#"INSERT OR REPLACE INTO sources (provider, path, dev, ino, size, mtime_ns,
            "offset", parser_version, fold_version, malformed_lines, digest, thread)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
        params![
            adapter.provider(),
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
    Ok(stat.size.saturating_sub(progress.offset))
}

/// Where to read a file whose stat is now `stat` from, given what the index
/// `stored` about it; `None` when it is up to date. A file that only grew is
/// read on from its last complete line; anything else is read again from
/// the start.
fn progress(stored: Option<Stored>, stat: &Stat) -> Option<Progress> {
    match stored {
        Some(Stored {
            stat: old,
            resume: Some(_),
        }) if old == *stat => None,
        Some(Stored {
            stat: old,
            resume: Some(progress),
        }) if (old.dev, old.ino) == (stat.dev, stat.ino) && stat.size >= progress.offset => {
            Some(progress)
        }
        _ => Some(Progress::default()),
    }
}

/// `file`'s identity, size and mtime now; `None` when it can't be read.
/// Taken before reading, so a file that grows during the read is seen as
/// changed next time.
fn stat(file: &Path) -> Option<Stat> {
    let metadata = fs::metadata(file).ok()?;
    Some(Stat {
        dev: metadata.dev(),
        ino: metadata.ino(),
        size: metadata.size(),
        mtime_ns: nanos(metadata.modified().ok()?),
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
        // A busy index is one another refresh is using, not a broken one.
        if is_busy(error.as_ref()) {
            return Err(error);
        }
        remove(path)?;
        open_as_is(path)
    })
}

impl DataError {
    /// Whether another process held the index past `BUSY_TIMEOUT`.
    pub(crate) fn is_busy(&self) -> bool {
        is_busy(self.source.as_ref())
    }
}

fn is_busy(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<rusqlite::Error>()
        .and_then(rusqlite::Error::sqlite_error_code)
        .is_some_and(|code| matches!(code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
}

fn open_as_is(path: &Path) -> Result<Connection, Cause> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    use_wal(&connection)?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    match version {
        SCHEMA_VERSION => {}
        // A new database; `IF NOT EXISTS` lets two first refreshes race safely.
        0 => {
            connection.execute_batch(SCHEMA)?;
            connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        _ => return Err(format!("the index has schema version {version}").into()),
    }
    Ok(connection)
}

/// Switches `connection` to WAL, waiting up to `BUSY_TIMEOUT` while another
/// connection switches it too: SQLite fails that switch at once rather than
/// calling the busy handler, so two first launches would otherwise race.
fn use_wal(connection: &Connection) -> rusqlite::Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match connection.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())) {
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            switched => return switched,
        }
    }
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

/// Every source of `adapter` the index has read, by path.
fn stored_sources(
    connection: &Connection,
    adapter: &dyn Adapter,
) -> Result<HashMap<PathBuf, Stored>, Cause> {
    let mut statement = connection.prepare(&format!(
        "SELECT {STORED_COLUMNS} FROM sources WHERE provider = ?1"
    ))?;
    let rows = statement.query_map([adapter.provider()], |row| stored(row, adapter))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What the index has read of `file`, a source of `adapter`, as of now.
fn stored_source(
    connection: &Connection,
    adapter: &dyn Adapter,
    file: &Path,
) -> Result<Option<Stored>, Cause> {
    let stored = connection
        .query_row(
            &format!("SELECT {STORED_COLUMNS} FROM sources WHERE path = ?1"),
            [file.as_os_str().as_bytes()],
            |row| stored(row, adapter),
        )
        .optional()?;
    Ok(stored.map(|(_, stored)| stored))
}

/// The `sources` columns `stored` reads, in its order.
const STORED_COLUMNS: &str = r#"path, dev, ino, size, mtime_ns, "offset", malformed_lines,
    digest, parser_version, fold_version"#;

/// A `sources` row of `adapter`'s, selected as `STORED_COLUMNS`, by path.
fn stored(row: &rusqlite::Row, adapter: &dyn Adapter) -> rusqlite::Result<(PathBuf, Stored)> {
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
    let mut said_folded = String::new();
    let mut said_as_typed = String::new();
    let mut tool_folded = String::new();
    for (seq, message) in (next..).zip(messages) {
        let folded = query::fold(&message.text);
        statement.execute(params![source, seq, role_name(message.role), message.text])?;
        searchable.execute(params![transaction.last_insert_rowid(), folded])?;
        // Each message ends in a newline, so no word runs into the next.
        if said(message.role) {
            said_folded.extend([folded.as_str(), "\n"]);
            said_as_typed.extend([message.text.as_str(), "\n"]);
        } else {
            tool_folded.extend([folded.as_str(), "\n"]);
        }
    }
    let mut document = transaction
        .prepare("INSERT INTO documents (source_path, kind, text) VALUES (?1, ?2, ?3)")?;
    let documents = [
        ("said", said_folded),
        ("said as typed", said_as_typed),
        ("tool", tool_folded),
    ];
    for (kind, text) in documents {
        if !text.is_empty() {
            document.execute(params![source, kind, text])?;
        }
    }
    Ok(())
}

/// Deletes everything stored from `source`'s messages; succeeds when there
/// is none.
fn forget_messages(transaction: &Transaction, source: &[u8]) -> Result<(), Cause> {
    transaction.execute("DELETE FROM messages WHERE source_path = ?1", [source])?;
    transaction.execute("DELETE FROM documents WHERE source_path = ?1", [source])?;
    Ok(())
}

/// How `role` is stored in the `messages` table.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::ToolCall => "tool_call",
        Role::ToolOutput => "tool_output",
    }
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
        "INSERT INTO sessions (provider, id, title, title_fold, working_dir,
            other_working_dirs, last_active_ns, branch, repository_url, recap, recap_fold,
            linked_prs, subagent_count, resume_program, resume_args, resume_cwd)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut searchable = transaction
        .prepare("INSERT INTO session_text (rowid, title, recap) VALUES (?1, ?2, ?3)")?;
    for session in sessions {
        let other_working_dirs: Vec<String> = session
            .other_working_dirs
            .iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect();
        let title_fold = query::fold(&session.title);
        let recap_fold = session.recap.as_deref().map(query::fold);
        insert.execute(params![
            provider,
            session.id,
            session.title,
            title_fold,
            session.working_dir.to_string_lossy(),
            serde_json::to_string(&other_working_dirs)?,
            nanos(session.last_active),
            session.recorded_git.branch,
            session.recorded_git.repository_url,
            session.recap,
            recap_fold,
            serde_json::to_string(&session.linked_prs)?,
            session.subagent_count as i64,
            session.resume.program,
            serde_json::to_string(&session.resume.args)?,
            session.resume.cwd.to_string_lossy(),
        ])?;
        searchable.execute(params![
            transaction.last_insert_rowid(),
            title_fold,
            recap_fold
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

/// Which of `candidates` match `word` in their title, recap or a message in
/// scope: tool-level text and subagents' threads count only with `tools`.
/// Only the rows of `candidates` are read, so none are when it is empty.
fn sessions_matching(
    connection: &Connection,
    word: &Word,
    tools: bool,
    candidates: &HashSet<SessionKey>,
) -> Result<HashSet<SessionKey>, Cause> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let in_play: Vec<[&str; 2]> = candidates
        .iter()
        .map(|key| [key.provider.as_str(), key.id.as_str()])
        .collect();
    let in_play = serde_json::to_string(&in_play)?;
    // Trigram rows hold three-character sequences, so a shorter word
    // can't be looked up in them.
    match word.key().chars().count() >= 3 {
        true => by_trigrams(connection, word, tools, &in_play),
        false => by_documents(connection, word, tools, &in_play),
    }
}

/// `sessions_matching` for a word of three or more characters: the trigram
/// tables give the rows that may hold it, and `Word::matches` checks each.
fn by_trigrams(
    connection: &Connection,
    word: &Word,
    tools: bool,
    in_play: &str,
) -> Result<HashSet<SessionKey>, Cause> {
    let scope = match tools {
        true => "true".to_string(),
        false => format!("{SAID} AND {OWN_THREAD}"),
    };
    let mut matched = HashSet::new();
    let mut statement = connection.prepare(&format!(
        "SELECT sessions.provider, sessions.id, sessions.title, sessions.recap
        FROM session_text JOIN sessions ON sessions.row_id = session_text.rowid
        WHERE (sessions.provider, sessions.id) IN ({IN_PLAY})
            AND session_text MATCH {PHRASE}"
    ))?;
    let mut rows = statement.query(params![in_play, word.key()])?;
    while let Some(row) = rows.next()? {
        let key = key_from_row(row)?;
        let title: String = row.get(2)?;
        let recap: Option<String> = row.get(3)?;
        if !matched.contains(&key)
            && (word.matches(&title) || recap.is_some_and(|recap| word.matches(&recap)))
        {
            matched.insert(key);
        }
    }
    // Each hit is checked against the sessions in play; the unary `+` stops
    // SQLite probing the threads index once per session in play for every hit.
    let mut statement = connection.prepare(&format!(
        "SELECT threads.provider, threads.session_id, messages.text
        FROM message_text JOIN messages ON messages.id = message_text.rowid
        JOIN sources ON sources.path = messages.source_path
        JOIN threads ON threads.provider = sources.provider
            AND threads.thread = sources.thread
        WHERE {scope} AND (+threads.provider, +threads.session_id) IN ({IN_PLAY})
            AND message_text MATCH {PHRASE}"
    ))?;
    let mut rows = statement.query(params![in_play, word.key()])?;
    while let Some(row) = rows.next()? {
        let key = key_from_row(row)?;
        if !matched.contains(&key) && word.matches(&row.get::<_, String>(2)?) {
            matched.insert(key);
        }
    }
    Ok(matched)
}

/// `sessions_matching` for a word of one or two characters, matched in SQL
/// against the stored folds, or the text as typed when case counts.
fn by_documents(
    connection: &Connection,
    word: &Word,
    tools: bool,
    in_play: &str,
) -> Result<HashSet<SessionKey>, Cause> {
    let typed = word.as_typed();
    let needle = typed.unwrap_or(word.key());
    let key = word.key();
    // `IN_PLAY` takes the sessions in play as `?1`.
    let mut bound: Vec<(&str, &dyn ToSql)> = vec![("?1", &in_play), (":needle", &needle)];
    let (title, recap) = match typed {
        Some(_) => ("title", "recap"),
        None => ("title_fold", "recap_fold"),
    };
    let mut matched = HashSet::new();
    let mut statement = connection.prepare(&format!(
        "SELECT provider, id FROM sessions
        WHERE (provider, id) IN ({IN_PLAY})
            AND (instr({title}, :needle) > 0 OR instr(coalesce({recap}, ''), :needle) > 0)"
    ))?;
    for session in statement.query_map(bound.as_slice(), key_from_row)? {
        matched.insert(session?);
    }
    let documents_in_scope = match (typed, tools) {
        (None, false) => format!(
            "{OWN_THREAD} AND documents.kind = 'said' AND instr(documents.text, :needle) > 0"
        ),
        (Some(_), false) => format!(
            "{OWN_THREAD} AND documents.kind = 'said as typed'
                AND instr(documents.text, :needle) > 0"
        ),
        (None, true) => "documents.kind IN ('said', 'tool')
            AND instr(documents.text, :needle) > 0"
            .to_string(),
        // Tool-level text isn't stored as typed, so its folded document picks
        // the sources that may hold the word, and their messages decide.
        (Some(_), true) => {
            bound.push((":key", &key));
            format!(
                "(documents.kind = 'said as typed' AND instr(documents.text, :needle) > 0)
                OR (documents.kind = 'tool' AND instr(documents.text, :key) > 0
                    AND EXISTS (SELECT 1 FROM messages
                        WHERE messages.source_path = documents.source_path
                            AND NOT ({SAID}) AND instr(messages.text, :needle) > 0))"
            )
        }
    };
    // The unary `+` keeps SQLite from starting at the sessions in play: it
    // would then read their documents out of storage order, several
    // times slower than reading every document in order and checking each.
    let mut statement = connection.prepare(&format!(
        "SELECT DISTINCT threads.provider, threads.session_id
        FROM threads
        JOIN sources ON sources.provider = threads.provider
            AND sources.thread = threads.thread
        JOIN documents ON documents.source_path = sources.path
        WHERE (+threads.provider, +threads.session_id) IN ({IN_PLAY})
            AND ({documents_in_scope})"
    ))?;
    for session in statement.query_map(bound.as_slice(), key_from_row)? {
        matched.insert(session?);
    }
    Ok(matched)
}

/// `provider`'s sessions as the index stores them.
fn stored_sessions(
    transaction: &Transaction,
    provider: &'static str,
) -> Result<Vec<Session>, Cause> {
    let mut statement = transaction.prepare(
        "SELECT id, title, working_dir, other_working_dirs, last_active_ns, branch,
            repository_url, recap, linked_prs, subagent_count, resume_program, resume_args,
            resume_cwd
        FROM sessions WHERE provider = ?1",
    )?;
    let mut rows = statement.query([provider])?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next()? {
        let other_working_dirs: Vec<String> = serde_json::from_str(&row.get::<_, String>(3)?)?;
        sessions.push(Session {
            provider,
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

        fn is_running(&self, _id: &str) -> bool {
            false
        }

        fn resumes_by_directory(&self) -> bool {
            false
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
        refresh(&[&stub], &index).unwrap();
        assert_eq!(refresh(&[&stub], &index).unwrap().files_read, 0);

        stub.version.set(2);
        let refreshed = refresh(&[&stub], &index).unwrap();
        assert_eq!(refreshed.files_read, 2);
        let titles: Vec<&str> = refreshed
            .sessions
            .iter()
            .map(|session| session.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["Read by parser version 2", "Read by parser version 2"]
        );
    }
}
