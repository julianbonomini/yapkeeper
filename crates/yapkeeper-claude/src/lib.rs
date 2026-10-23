//! The Claude Code adapter. It owns where Claude Code keeps its history, the
//! JSONL grammar of a transcript, which text counts as a prompt, how a title
//! is chosen and how a session is resumed.

#![forbid(unsafe_code)]

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Message, MissingHome, RecordedGit, ResumeCommand, Role, Scan, Session, read_jsonl,
};

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
    let mut messages = Vec::new();
    let mut working_dirs: Vec<PathBuf> = Vec::new();
    let mut latest_timestamp = None;
    let mut custom_title = None;
    let mut ai_title = None;
    let mut branch = None;
    let read_to_end = read_jsonl(path, malformed_lines, |record| {
        if let Some(time) = record["timestamp"].as_str().and_then(parse_timestamp) {
            latest_timestamp = latest_timestamp.max(Some(time));
        }
        let role = match record["type"].as_str() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("custom-title") => {
                custom_title = non_empty(&record["customTitle"]).or(custom_title.take());
                return ControlFlow::Continue(());
            }
            Some("ai-title") => {
                ai_title = non_empty(&record["aiTitle"]).or(ai_title.take());
                return ControlFlow::Continue(());
            }
            _ => return ControlFlow::Continue(()),
        };
        if record["isMeta"].as_bool() == Some(true) || record["isSidechain"].as_bool() == Some(true)
        {
            return ControlFlow::Continue(());
        }
        if let Some(cwd) = record["cwd"].as_str().map(PathBuf::from)
            && !working_dirs.contains(&cwd)
        {
            working_dirs.push(cwd);
        }
        if branch.is_none() {
            branch = non_empty(&record["gitBranch"]);
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
        ControlFlow::Continue(())
    });
    if !read_to_end {
        return None;
    }
    let last_active = match latest_timestamp {
        Some(time) => time,
        None => fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()?,
    };
    if working_dirs.is_empty() {
        return None;
    }
    let working_dir = working_dirs.remove(0);
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
        other_working_dirs: working_dirs,
        last_active,
        recorded_git: RecordedGit {
            branch,
            repository_url: None,
        },
        messages,
    })
}

fn push_message(messages: &mut Vec<Message>, role: Role, text: &str) {
    if role == Role::User && is_injected(text.trim()) {
        return;
    }
    messages.extend(Message::new(role, text));
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
    yapkeeper_model::root_from_env("CLAUDE_CONFIG_DIR", ".claude")
}
