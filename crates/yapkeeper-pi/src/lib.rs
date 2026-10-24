//! The Pi adapter. It owns where Pi keeps its session files, which of them
//! are top-level sessions, the JSONL grammar of a session file, and how a
//! session is resumed. The model folds what it reads into sessions.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Fact, Message, MissingHome, Reading, ResumeCommand, Role, ThreadKind, TitleKind,
    jsonl_files, non_empty_text, read_jsonl_from, record_time,
};

const PARSER_VERSION: u32 = 1;

/// Pi's sessions under one session directory.
pub struct Pi {
    root: PathBuf,
}

impl Pi {
    /// Reads the top-level session files under `root`: those in `root`
    /// itself, as a custom session directory holds them, and those one
    /// folder down, as Pi's default `<cwd>` folders hold them.
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
        // Subagent runs and forks sit deeper, in a folder named after their
        // session file, so they are never listed as sessions of their own.
        // A `subagent-artifacts` transcript one level down has no `session`
        // header, so it gives no session either.
        let mut files = jsonl_files(&self.root, 0);
        files.extend(jsonl_files(&self.root, 1));
        files
    }

    fn read(&self, source: &Path, from: u64) -> Reading {
        let mut facts = Vec::new();
        let mut malformed_lines = 0;
        let offset = read_jsonl_from(source, from, &mut malformed_lines, |record| {
            read_record(&mut facts, &record);
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

/// The facts one session record gives.
fn read_record(facts: &mut Vec<Fact>, record: &Value) {
    let Some(kind) = record["type"].as_str() else {
        return;
    };
    if let Some(time) = record_time(record) {
        facts.push(Fact::Activity(time));
    }
    match kind {
        // A fork's header names its `parentSession` too, but a top-level
        // fork is a separate session the person went on with.
        "session" => {
            facts.push(Fact::Identity {
                id: record["id"].as_str().map(str::to_string),
                kind: ThreadKind::Session,
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
    match std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => yapkeeper_model::root_from_env("PI_CODING_AGENT_DIR", ".pi/agent")
            .map(|agent_dir| agent_dir.join("sessions")),
    }
}
