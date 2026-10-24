//! The session model every provider adapter feeds, the contract an adapter
//! fulfils, the fold rules that turn an adapter's facts into sessions
//! (including which session each thread belongs to), and the JSONL reading
//! rules adapters share. It never holds a session's messages: they stay in
//! each `Reading`'s facts, and `Assembled::session_of` says which session the
//! thread that recorded them belongs to. Nothing here knows how any provider
//! stores its history.

#![forbid(unsafe_code)]

mod assemble;

use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

pub use assemble::{Assembled, FOLD_VERSION, SourceDigest, assemble};

// Caps keep memory small on long transcripts; a tool call's input and what
// a tool gave back matter far less than what the person and the assistant said.
const MAX_MESSAGE_BYTES: usize = 4_000;
const MAX_TOOL_BYTES: usize = 300;

/// A provider's history, read without modifying it.
pub trait Adapter {
    /// Such as "claude"; stored with every session so the model never lists providers.
    fn provider(&self) -> &'static str;
    /// Bump when `read` would give different facts from the same bytes.
    fn parser_version(&self) -> u32;
    /// The directory this adapter reads its history under. Touches nothing on
    /// disk.
    fn history_dir(&self) -> PathBuf;
    /// Every history file to read, under `history_dir`. Lists directories but
    /// opens no file; a missing directory gives no files.
    fn sources(&self) -> Vec<PathBuf>;
    /// The facts in `source` from byte `from` to the end of its last complete
    /// line, in file order, and the offset reached. `from` is 0 or an offset
    /// this method returned for the same file; reading on from that offset
    /// gives the same facts as one read from 0. Never fails: an unreadable
    /// file gives no facts and offset `from`.
    fn read(&self, source: &Path, from: u64) -> Reading;
    /// The exact process that resumes session `id` in `working_dir`.
    fn resume(&self, id: &str, working_dir: &Path) -> ResumeCommand;
    /// Whether the provider's own metadata says session `id` is open right
    /// now. Reads at call time and keeps nothing. Never fails: metadata that
    /// is missing or can't be read means not running.
    fn is_running(&self, id: &str) -> bool;
    /// Whether the provider finds a session by the directory it is started
    /// in, so resuming it from another directory may not find it.
    fn resumes_by_directory(&self) -> bool;
}

/// What one call to `Adapter::read` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub facts: Vec<Fact>,
    pub offset: u64,
    /// Malformed complete lines between `from` and `offset`.
    pub malformed_lines: usize,
}

/// One thing a history file records, folded across lines and files by the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// The thread this file records; the first in a file wins. `id: None`
    /// means the file has no usable identity, and it is left out.
    Identity {
        id: Option<String>,
        kind: ThreadKind,
    },
    /// Each kept once, in order; the first is the working directory.
    WorkingDir(PathBuf),
    /// The latest wins; without one, the file's mtime.
    Activity(SystemTime),
    /// In file order; a session's first User message is its fallback title.
    Message(Message),
    /// `thread: None` is this file's thread. Named beats Generated beats the
    /// first prompt; the latest of a kind wins.
    Title {
        thread: Option<String>,
        kind: TitleKind,
        text: String,
    },
    /// The latest wins.
    Recap(String),
    /// Each kept once, in order.
    LinkedPr(u64),
    /// The first wins.
    Git(RecordedGit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadKind {
    Session,
    /// Belongs to the session at the top of its parent chain; left out when
    /// `parent` is `None` or the chain reaches no session.
    Subagent {
        parent: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleKind {
    /// Chosen by the person.
    Named,
    /// Written by the provider.
    Generated,
}

/// A conversation recorded by a provider, ready to show and resume, without its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Supplied by the adapter, such as "claude", so the model never lists providers.
    pub provider: &'static str,
    pub id: String,
    /// Never empty.
    pub title: String,
    /// The first one recorded.
    pub working_dir: PathBuf,
    /// Directories recorded after `working_dir`, each once, in the order
    /// first seen, for the preview.
    pub other_working_dirs: Vec<PathBuf>,
    pub last_active: SystemTime,
    /// What the provider recorded about Git when the session started.
    pub recorded_git: RecordedGit,
    /// The latest summary the provider wrote of the session, when it wrote one.
    pub recap: Option<String>,
    /// Numbers of the pull requests the provider linked to this session, each
    /// once, in the order first linked.
    pub linked_prs: Vec<u64>,
    /// Agents this session started, through any chain of subagents.
    pub subagent_count: usize,
    pub resume: ResumeCommand,
}

/// Git facts the provider recorded when the session ran. They may no longer
/// match the disk, and either may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedGit {
    /// Such as `feature/billing-retry`.
    pub branch: Option<String>,
    /// The remote, such as `git@example.com:acme/acme-api.git`.
    pub repository_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

impl Message {
    /// `text` trimmed and cut to the length kept for `role`, ending in `…`
    /// when cut, or `None` when nothing but whitespace is left.
    pub fn new(role: Role, text: &str) -> Option<Message> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let max_bytes = match role {
            Role::User | Role::Assistant => MAX_MESSAGE_BYTES,
            Role::ToolCall | Role::ToolOutput => MAX_TOOL_BYTES,
        };
        let text = if text.len() <= max_bytes {
            text.to_string()
        } else {
            let mut end = max_bytes;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &text[..end])
        };
        Some(Message { role, text })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A tool the assistant called, kept apart from ordinary messages.
    ToolCall,
    /// What a tool gave back, kept apart from ordinary messages.
    ToolOutput,
}

/// The exact process to start to resume a session. Nothing in this crate runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// The directory `$<variable>` names, or `None` when it is unset or empty.
pub fn dir_from_env(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// A provider's history directory for this process: `$<variable>` when it is
/// set and non-empty, otherwise `$HOME/<home_subdir>`.
pub fn root_from_env(variable: &'static str, home_subdir: &str) -> Result<PathBuf, MissingHome> {
    dir_from_env(variable)
        .or_else(|| dir_from_env("HOME").map(|home| home.join(home_subdir)))
        .ok_or(MissingHome { variable })
}

/// Neither a directory's own variable nor `HOME` names a directory.
#[derive(Debug)]
pub struct MissingHome {
    /// The directory's own variable, such as `CLAUDE_CONFIG_DIR` or
    /// `XDG_DATA_HOME`.
    pub variable: &'static str,
}

impl fmt::Display for MissingHome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "neither {} nor HOME is set", self.variable)
    }
}

impl std::error::Error for MissingHome {}

/// The trimmed string `value` holds, or `None` when it holds no string or
/// only whitespace.
pub fn non_empty_text(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// When `record` was written, from its RFC 3339 `timestamp`, or `None` when
/// it has none that parses.
pub fn record_time(record: &Value) -> Option<SystemTime> {
    let text = record["timestamp"].as_str()?;
    text.parse::<jiff::Timestamp>().ok().map(SystemTime::from)
}

/// `.jsonl` files exactly `depth` directory levels below `dir`, or none when
/// `dir` can't be read.
pub fn jsonl_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if depth > 0 {
            files.extend(jsonl_files(&path, depth - 1));
        } else if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    files
}

/// Hands each complete JSONL record from byte `from` onward to `on_record`,
/// adding each malformed complete line to `malformed_lines`, and returns the
/// offset just past the last complete line. An unfinished final line may
/// still be being written, so it is neither handed over nor counted. A read
/// error stops early and returns the offset reached.
pub fn read_jsonl_from(
    path: &Path,
    from: u64,
    malformed_lines: &mut usize,
    mut on_record: impl FnMut(Value),
) -> u64 {
    let Ok(mut file) = fs::File::open(path) else {
        return from;
    };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return from;
    }
    let mut reader = BufReader::new(file);
    let mut offset = from;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).is_err() || line.last() != Some(&b'\n') {
            return offset;
        }
        offset += line.len() as u64;
        match serde_json::from_slice::<Value>(&line) {
            Ok(record) => on_record(record),
            Err(_) => *malformed_lines += 1,
        }
    }
}
