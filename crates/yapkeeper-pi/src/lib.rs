//! The Pi adapter. It owns where Pi keeps its session files, which of them
//! are sessions and which are a session's subagent runs and forks, the JSONL
//! grammar of a session file, and how a session is resumed. The model folds
//! what it reads into sessions.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Fact, Message, MissingHome, Reading, ResumeCommand, Role, ThreadKind, TitleKind,
    jsonl_files, non_empty_text, read_jsonl_from, record_time,
};

const PARSER_VERSION: u32 = 2;

/// Pi's sessions under one session directory.
pub struct Pi {
    root: PathBuf,
}

impl Pi {
    /// Reads the top-level session files under `root`: those in `root`
    /// itself, as a custom session directory holds them, and those one
    /// folder down, as Pi's default `<cwd>` folders hold them, with the
    /// subagent runs and forks in each session's folder.
    pub fn at(root: PathBuf) -> Pi {
        Pi { root }
    }
}

impl Adapter for Pi {
    fn provider(&self) -> &'static str {
        "pi"
    }

    fn parser_version(&self) -> u32 {
        PARSER_VERSION
    }

    fn history_dir(&self) -> PathBuf {
        self.root.clone()
    }

    fn sources(&self) -> Vec<PathBuf> {
        // `subagent-artifacts` transcripts beside the sessions are found too,
        // but have no `session` header, so they give no thread.
        let session_files = jsonl_files(&self.root, 0)
            .into_iter()
            .chain(jsonl_files(&self.root, 1));
        let mut files = Vec::new();
        for session_file in session_files {
            // A run or fork whose session file is gone is never found.
            let folder = session_file.with_extension("");
            files.extend(
                [jsonl_files(&folder, 1), jsonl_files(&folder, 2)]
                    .into_iter()
                    .flatten()
                    .filter(|nested| session_folder_of(nested) == Some(folder.as_path())),
            );
            files.push(session_file);
        }
        files
    }

    fn read(&self, source: &Path, from: u64) -> Reading {
        // Runs and forks copy the session header's shape, so where the file
        // sits is what says whose subagent it is. Only the path below the
        // root counts, whatever the root itself is named.
        let below_root = source.strip_prefix(&self.root).unwrap_or(source);
        let thread = match session_folder_of(below_root) {
            None => ThreadKind::Session,
            Some(folder) => ThreadKind::Subagent {
                // Named after its session file, `<timestamp>_<session-id>`.
                parent: folder
                    .file_name()
                    .and_then(|name| name.to_str()?.split_once('_'))
                    .map(|(_timestamp, id)| id.to_string()),
            },
        };
        let mut facts = Vec::new();
        let mut malformed_lines = 0;
        let offset = read_jsonl_from(source, from, &mut malformed_lines, |record| {
            read_record(&mut facts, &record, &thread);
        });
        Reading {
            facts,
            offset,
            malformed_lines,
        }
    }

    fn resume(&self, id: &str, working_dir: &Path) -> ResumeCommand {
        ResumeCommand {
            program: "pi".to_string(),
            args: vec!["--session".to_string(), id.to_string()],
            cwd: working_dir.to_path_buf(),
        }
    }

    fn is_running(&self, _id: &str) -> bool {
        // Pi keeps no record of which sessions are open.
        false
    }

    fn resumes_by_directory(&self) -> bool {
        // `pi --session <id>` looks in the starting directory's sessions
        // first, and offers to fork a session it finds elsewhere.
        true
    }
}

/// The facts one record gives, in a file that records a `thread`.
fn read_record(facts: &mut Vec<Fact>, record: &Value, thread: &ThreadKind) {
    let Some(kind) = record["type"].as_str() else {
        return;
    };
    if let Some(time) = record_time(record) {
        facts.push(Fact::Activity(time));
    }
    match kind {
        "session" => {
            facts.push(Fact::Identity {
                id: record["id"].as_str().map(str::to_string),
                kind: thread.clone(),
            });
            if let Some(cwd) = record["cwd"].as_str() {
                facts.push(Fact::WorkingDir(PathBuf::from(cwd)));
            }
        }
        "session_info" => {
            if let Some(name) = non_empty_text(&record["name"]) {
                facts.push(Fact::Title {
                    thread: None,
                    kind: TitleKind::Named,
                    text: name,
                });
            }
        }
        "message" => read_message(facts, &record["message"]),
        // An extension's notice, such as a background task finishing, that
        // Pi hands the model as context; neither side of the conversation
        // wrote it.
        "custom_message" => {
            for text in texts(&record["content"]) {
                facts.extend(Message::new(Role::ToolOutput, text).map(Fact::Message));
            }
        }
        _ => {}
    }
}

/// The facts one `message` record's message gives. Thinking is never kept.
fn read_message(facts: &mut Vec<Fact>, message: &Value) {
    let content = &message["content"];
    let mut push = |role, text: &str| facts.extend(Message::new(role, text).map(Fact::Message));
    match message["role"].as_str() {
        Some("user") => texts(content).for_each(|text| push(Role::User, text)),
        Some("assistant") => {
            for block in content.as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => push(Role::Assistant, block["text"].as_str().unwrap_or("")),
                    Some("toolCall") => {
                        let call = format!(
                            "{} {}",
                            block["name"].as_str().unwrap_or("tool"),
                            block["arguments"]
                        );
                        push(Role::ToolCall, &call);
                    }
                    _ => {}
                }
            }
        }
        Some("toolResult") => texts(content).for_each(|text| push(Role::ToolOutput, text)),
        // A command the person ran with `!`, outside the assistant's turn.
        Some("bashExecution") => {
            push(Role::ToolCall, message["command"].as_str().unwrap_or(""));
            push(Role::ToolOutput, message["output"].as_str().unwrap_or(""));
        }
        _ => {}
    }
}

/// The folder of the session whose subagent `file` records: a fork sits at
/// `<folder>/forks/*.jsonl` and a run at `<folder>/<run-id>/run-<n>/session.jsonl`.
/// `None` for any other file, such as a session file.
fn session_folder_of(file: &Path) -> Option<&Path> {
    let dir = file.parent()?;
    let dir_name = dir.file_name()?.to_str()?;
    if dir_name == "forks" {
        dir.parent()
    } else if dir_name.starts_with("run-") && file.file_name()? == "session.jsonl" {
        dir.parent()?.parent()
    } else {
        None
    }
}

/// The text of `content`, which Pi records either as a string or as blocks,
/// of which only `text` blocks hold text.
fn texts(content: &Value) -> impl Iterator<Item = &str> {
    let blocks = content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str());
    content.as_str().into_iter().chain(blocks)
}

/// Pi's session directory for this process: `$PI_CODING_AGENT_SESSION_DIR`
/// when it is set and non-empty, otherwise `$PI_CODING_AGENT_DIR/sessions`,
/// otherwise `$HOME/.pi/agent/sessions`.
pub fn session_dir() -> Result<PathBuf, MissingHome> {
    match yapkeeper_model::dir_from_env("PI_CODING_AGENT_SESSION_DIR") {
        Some(dir) => Ok(dir),
        None => yapkeeper_model::root_from_env("PI_CODING_AGENT_DIR", ".pi/agent")
            .map(|agent_dir| agent_dir.join("sessions")),
    }
}
