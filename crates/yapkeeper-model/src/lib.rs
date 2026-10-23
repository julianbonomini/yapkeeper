//! The session model every provider adapter hands back, and the contract an
//! adapter fulfils. Nothing here knows how any provider stores its history.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::SystemTime;

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
    pub last_active: SystemTime,
    /// In transcript order, with injected context removed and long text capped.
    pub messages: Vec<Message>,
    pub resume: ResumeCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
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
