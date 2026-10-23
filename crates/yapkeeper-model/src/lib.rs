//! The session model every provider adapter hands back, the contract an
//! adapter fulfils, and the JSONL reading rules adapters share. Nothing here
//! knows how any provider stores its history.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

// Caps keep memory small on long transcripts; a tool call's input matters
// far less than what the person and the assistant said.
const MAX_MESSAGE_BYTES: usize = 4_000;
const MAX_TOOL_CALL_BYTES: usize = 300;

/// A provider's history, read without modifying it.
pub trait Adapter {
    /// Every top-level session under this adapter's root, in no particular
    /// order. Never fails: a missing root gives an empty Scan, unreadable
    /// files and sessions without a prompt or working directory are left
    /// out, and malformed complete lines are counted.
    fn scan(&self) -> Scan;
}

/// One adapter's results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub sessions: Vec<Session>,
    pub malformed_lines: usize,
    /// The directory looked in, for the empty-state message.
    pub searched: PathBuf,
}

/// A conversation recorded by a provider, ready to show and resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Supplied by the adapter, such as "claude", so the model never lists providers.
    pub provider: &'static str,
    pub id: String,
    /// Already resolved by the adapter, and never empty.
    pub title: String,
    /// The first one recorded.
    pub working_dir: PathBuf,
    /// Directories recorded after `working_dir`, each once, in the order
    /// first seen, for the preview.
    pub other_working_dirs: Vec<PathBuf>,
    pub last_active: SystemTime,
    /// What the provider recorded about Git when the session started.
    pub recorded_git: RecordedGit,
    /// In transcript order, with injected context removed and long text capped.
    pub messages: Vec<Message>,
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
            Role::ToolCall => MAX_TOOL_CALL_BYTES,
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
}

/// The exact process to start to resume a session. Nothing in this crate runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// A provider's history directory for this process: `$<variable>` when it is
/// set and non-empty, otherwise `$HOME/<home_subdir>`.
pub fn root_from_env(variable: &'static str, home_subdir: &str) -> Result<PathBuf, MissingHome> {
    match std::env::var_os(variable) {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => Ok(PathBuf::from(home).join(home_subdir)),
            _ => Err(MissingHome { variable }),
        },
    }
}

/// Neither a provider's root variable nor `HOME` names a directory to read.
#[derive(Debug)]
pub struct MissingHome {
    /// The provider's own variable, such as `CLAUDE_CONFIG_DIR`.
    pub variable: &'static str,
}

impl fmt::Display for MissingHome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "neither {} nor HOME is set", self.variable)
    }
}

impl std::error::Error for MissingHome {}

/// Hands each complete JSONL record in `path` to `on_record` until it
/// breaks, adding each malformed complete line to `malformed_lines`. A final
/// line without its newline may still be being written, so it is neither
/// handed over nor counted. True when every complete record was handed over;
/// false when `on_record` broke or the file couldn't be read.
pub fn read_jsonl(
    path: &Path,
    malformed_lines: &mut usize,
    mut on_record: impl FnMut(Value) -> ControlFlow<()>,
) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).is_err() {
            return false;
        }
        if line.last() != Some(&b'\n') {
            return true;
        }
        match serde_json::from_slice::<Value>(&line) {
            Ok(record) => {
                if on_record(record).is_break() {
                    return false;
                }
            }
            Err(_) => *malformed_lines += 1,
        }
    }
}
