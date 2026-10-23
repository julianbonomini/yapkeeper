//! The Codex adapter. It owns where Codex keeps its rollouts, the JSONL
//! grammar of a rollout and of the session index, which rollouts are
//! subagents, how a title is chosen and how a session is resumed.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Message, MissingHome, ResumeCommand, Role, Scan, Session, read_jsonl,
};

/// Codex's history under one `CODEX_HOME`.
pub struct Codex {
    home: PathBuf,
}

impl Codex {
    /// Reads rollouts under `<home>/sessions` and `<home>/archived_sessions`,
    /// with titles from `<home>/session_index.jsonl`.
    pub fn at(home: PathBuf) -> Codex {
        Codex { home }
    }
}

impl Adapter for Codex {
    fn scan(&self) -> Scan {
        let mut malformed_lines = 0;
        let thread_names =
            read_thread_names(&self.home.join("session_index.jsonl"), &mut malformed_lines);
        // Active rollouts sit in `<year>/<month>/<day>/`; archived ones don't.
        let mut rollouts = jsonl_files(&self.home.join("sessions"), 3);
        rollouts.extend(jsonl_files(&self.home.join("archived_sessions"), 0));
        let sessions = rollouts
            .iter()
            .filter_map(|path| read_session(path, &thread_names, &mut malformed_lines))
            .collect();
        Scan {
            sessions,
            malformed_lines,
            searched: self.home.clone(),
        }
    }
}

/// `.jsonl` files exactly `depth` directory levels below `dir`, or none when
/// `dir` can't be read.
fn jsonl_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
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

/// Each session's `thread_name` from the session index. A session renamed
/// more than once appears on several lines, and the last one wins.
fn read_thread_names(path: &Path, malformed_lines: &mut usize) -> HashMap<String, String> {
    let mut thread_names = HashMap::new();
    read_jsonl(path, malformed_lines, |record| {
        if let (Some(id), Some(name)) = (record["id"].as_str(), non_empty(&record["thread_name"])) {
            thread_names.insert(id.to_string(), name);
        }
        ControlFlow::Continue(())
    });
    thread_names
}

/// The session recorded in one rollout, or `None` when it can't be read, is
/// a subagent's, or has nothing to resume. Adds each malformed complete line
/// read to `malformed_lines`.
fn read_session(
    path: &Path,
    thread_names: &HashMap<String, String>,
    malformed_lines: &mut usize,
) -> Option<Session> {
    let mut seen_meta = false;
    let mut id: Option<String> = None;
    let mut working_dirs: Vec<PathBuf> = Vec::new();
    let mut messages = Vec::new();
    let mut latest_timestamp = None;
    let read_to_end = read_jsonl(path, malformed_lines, |record| {
        if let Some(time) = record["timestamp"].as_str().and_then(parse_timestamp) {
            latest_timestamp = latest_timestamp.max(Some(time));
        }
        let payload = &record["payload"];
        match record["type"].as_str() {
            // A fork copies its parent's history, parent `session_meta`
            // included, after its own, so only the first names this session.
            Some("session_meta") => {
                if !seen_meta {
                    seen_meta = true;
                    if !payload["source"]["subagent"].is_null()
                        || !payload["parent_thread_id"].is_null()
                    {
                        return ControlFlow::Break(());
                    }
                    id = payload["id"].as_str().map(str::to_string);
                }
                if let Some(cwd) = payload["cwd"].as_str().map(PathBuf::from)
                    && !working_dirs.contains(&cwd)
                {
                    working_dirs.push(cwd);
                }
            }
            // `event_msg` records echo these messages, so only response
            // items are read.
            Some("response_item") => match payload["type"].as_str() {
                Some("message") => {
                    let role = match payload["role"].as_str() {
                        Some("user") => Role::User,
                        Some("assistant") => Role::Assistant,
                        _ => return ControlFlow::Continue(()),
                    };
                    for block in payload["content"].as_array().into_iter().flatten() {
                        let text = block["text"].as_str().unwrap_or("").trim();
                        if !(role == Role::User && is_injected(text)) {
                            messages.extend(Message::new(role, text));
                        }
                    }
                }
                Some(kind @ ("function_call" | "custom_tool_call")) => {
                    let input = match kind {
                        "function_call" => &payload["arguments"],
                        _ => &payload["input"],
                    };
                    let call = format!(
                        "{} {}",
                        payload["name"].as_str().unwrap_or("tool"),
                        input.as_str().unwrap_or("")
                    );
                    messages.extend(Message::new(Role::ToolCall, &call));
                }
                _ => {}
            },
            _ => {}
        }
        ControlFlow::Continue(())
    });
    if !read_to_end {
        return None;
    }
    let id = id?;
    if working_dirs.is_empty() {
        return None;
    }
    let working_dir = working_dirs.remove(0);
    let last_active = match latest_timestamp {
        Some(time) => time,
        None => fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()?,
    };
    let first_prompt = &messages
        .iter()
        .find(|message| message.role == Role::User)?
        .text;
    let title = thread_names.get(&id).unwrap_or(first_prompt).clone();
    Some(Session {
        provider: "codex",
        resume: ResumeCommand {
            program: "codex".to_string(),
            args: vec![
                "resume".to_string(),
                id.clone(),
                "-C".to_string(),
                working_dir.display().to_string(),
            ],
            cwd: working_dir.clone(),
        },
        id,
        title,
        working_dir,
        other_working_dirs: working_dirs,
        last_active,
        messages,
    })
}

/// Text Codex adds to a user turn on the user's behalf: environment and
/// instruction blocks, and the `AGENTS.md` it loaded.
fn is_injected(text: &str) -> bool {
    text.starts_with('<') || text.starts_with("# AGENTS.md")
}

fn non_empty(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_timestamp(text: &str) -> Option<SystemTime> {
    text.parse::<jiff::Timestamp>().ok().map(SystemTime::from)
}

/// Codex's home for this process: `$CODEX_HOME` when it is set and
/// non-empty, otherwise `$HOME/.codex`.
pub fn home_dir() -> Result<PathBuf, MissingHome> {
    yapkeeper_model::root_from_env("CODEX_HOME", ".codex")
}
