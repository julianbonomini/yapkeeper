//! The Claude Code adapter. It owns where Claude Code keeps its history, the
//! JSONL grammar of a transcript, which text counts as a prompt, how a title
//! is chosen and how a session is resumed.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use yapkeeper_model::{Adapter, Message, ResumeCommand, Role, Scan, Session};

// Caps keep memory small on long transcripts; a tool call's input matters
// far less than what the person and the assistant said.
const MAX_MESSAGE_BYTES: usize = 4_000;
const MAX_TOOL_CALL_BYTES: usize = 300;

/// Claude Code's history under one configuration directory.
pub struct ClaudeCode {
    config_dir: PathBuf,
}

impl ClaudeCode {
    /// Reads sessions under `<config_dir>/projects`.
    pub fn at(config_dir: PathBuf) -> ClaudeCode {
        ClaudeCode { config_dir }
    }
}

impl Adapter for ClaudeCode {
    fn scan(&self) -> Scan {
        let projects = self.config_dir.join("projects");
        let mut malformed_lines = 0;
        let sessions = transcript_files(&projects)
            .iter()
            .filter_map(|path| read_session(path, &mut malformed_lines))
            .collect();
        Scan {
            sessions,
            malformed_lines,
            searched: projects,
        }
    }
}

/// Top-level transcripts, `<projects>/<encoded-project>/<id>.jsonl`. Nothing
/// deeper is visited, so `<id>/subagents/` transcripts never become sessions.
fn transcript_files(projects: &Path) -> Vec<PathBuf> {
    let Ok(project_dirs) = fs::read_dir(projects) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for project_dir in project_dirs.filter_map(Result::ok) {
        let Ok(entries) = fs::read_dir(project_dir.path()) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
            {
                files.push(path);
            }
        }
    }
    files
}

/// The session recorded in one transcript file, or `None` when it can't be
/// read or has nothing to resume. Adds each malformed complete line to
/// `malformed_lines`, whether or not the session is kept.
fn read_session(path: &Path, malformed_lines: &mut usize) -> Option<Session> {
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut working_dir = None;
    let mut latest_timestamp = None;
    let mut custom_title = None;
    let mut ai_title = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        reader.read_until(b'\n', &mut line).ok()?;
        // A final line without its newline may still be being written, so
        // it is neither applied nor counted as malformed.
        if line.last() != Some(&b'\n') {
            break;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            *malformed_lines += 1;
            continue;
        };
        if let Some(time) = record["timestamp"].as_str().and_then(parse_timestamp) {
            latest_timestamp = latest_timestamp.max(Some(time));
        }
        let role = match record["type"].as_str() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("custom-title") => {
                custom_title = non_empty(&record["customTitle"]).or(custom_title);
                continue;
            }
            Some("ai-title") => {
                ai_title = non_empty(&record["aiTitle"]).or(ai_title);
                continue;
            }
            _ => continue,
        };
        if record["isMeta"].as_bool() == Some(true) || record["isSidechain"].as_bool() == Some(true)
        {
            continue;
        }
        if working_dir.is_none() {
            working_dir = record["cwd"].as_str().map(PathBuf::from);
        }
        match &record["message"]["content"] {
            Value::String(text) => push_message(&mut messages, role, text),
            Value::Array(blocks) => {
                for block in blocks {
                    match block["type"].as_str() {
                        Some("text") => {
                            push_message(&mut messages, role, block["text"].as_str().unwrap_or(""));
                        }
                        Some("tool_use") => {
                            let name = block["name"].as_str().unwrap_or("tool");
                            let call = format!("{name} {}", block["input"]);
                            push_message(&mut messages, Role::ToolCall, &call);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let last_active = match latest_timestamp {
        Some(time) => time,
        None => fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()?,
    };
    let working_dir = working_dir?;
    let first_prompt = messages
        .iter()
        .find(|message| message.role == Role::User)?
        .text
        .clone();
    let title = custom_title.or(ai_title).unwrap_or(first_prompt);
    Some(Session {
        provider: "claude",
        resume: ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), id.clone()],
            cwd: working_dir.clone(),
        },
        id,
        title,
        working_dir,
        last_active,
        messages,
    })
}

fn push_message(messages: &mut Vec<Message>, role: Role, text: &str) {
    let text = text.trim();
    if text.is_empty() || (role == Role::User && is_injected(text)) {
        return;
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
    messages.push(Message { role, text });
}

/// Text Claude Code adds to a user turn on the user's behalf: command
/// caveats and tags, instruction files and environment blocks.
fn is_injected(text: &str) -> bool {
    text.starts_with('<') || text.starts_with("Caveat:") || text.starts_with("# AGENTS.md")
}

fn non_empty(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_timestamp(text: &str) -> Option<SystemTime> {
    text.parse::<jiff::Timestamp>().ok().map(SystemTime::from)
}

/// Claude Code's configuration directory for this process: `$CLAUDE_CONFIG_DIR`
/// when it is set and non-empty, otherwise `$HOME/.claude`.
pub fn config_dir() -> Result<PathBuf, MissingHome> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => Ok(PathBuf::from(home).join(".claude")),
            _ => Err(MissingHome),
        },
    }
}

/// Neither `CLAUDE_CONFIG_DIR` nor `HOME` names a directory to read.
#[derive(Debug)]
pub struct MissingHome;

impl fmt::Display for MissingHome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("neither CLAUDE_CONFIG_DIR nor HOME is set")
    }
}

impl std::error::Error for MissingHome {}
