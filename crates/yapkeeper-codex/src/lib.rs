//! The Codex adapter. It owns where Codex keeps its rollouts, the JSONL
//! grammar of a rollout and of the session index, which rollouts are
//! subagents and whose, and how a session is resumed. The model folds what
//! it reads into sessions.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Fact, Message, MissingHome, Reading, RecordedGit, ResumeCommand, Role, ThreadKind,
    TitleKind, jsonl_files, read_jsonl_from,
};

const PARSER_VERSION: u32 = 2;

const SESSION_INDEX: &str = "session_index.jsonl";

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
    fn provider(&self) -> &'static str {
        "codex"
    }

    fn parser_version(&self) -> u32 {
        PARSER_VERSION
    }

    fn history_dir(&self) -> PathBuf {
        self.home.clone()
    }

    fn sources(&self) -> Vec<PathBuf> {
        // Active rollouts sit in `<year>/<month>/<day>/`; archived ones don't.
        let mut files = jsonl_files(&self.home.join("sessions"), 3);
        files.extend(jsonl_files(&self.home.join("archived_sessions"), 0));
        let session_index = self.home.join(SESSION_INDEX);
        if session_index.is_file() {
            files.push(session_index);
        }
        files
    }

    fn read(&self, source: &Path, from: u64) -> Reading {
        let mut facts = Vec::new();
        let mut malformed_lines = 0;
        let offset = if source.file_name().is_some_and(|name| name == SESSION_INDEX) {
            // A session renamed more than once appears on several lines, and
            // the model keeps the latest.
            read_jsonl_from(source, from, &mut malformed_lines, |record| {
                if let (Some(id), Some(name)) =
                    (record["id"].as_str(), non_empty(&record["thread_name"]))
                {
                    facts.push(Fact::Title {
                        thread: Some(id.to_string()),
                        kind: TitleKind::Named,
                        text: name,
                    });
                }
            })
        } else {
            read_jsonl_from(source, from, &mut malformed_lines, |record| {
                read_rollout_record(&mut facts, &record);
            })
        };
        Reading {
            facts,
            offset,
            malformed_lines,
        }
    }

    fn resume(&self, id: &str, working_dir: &Path) -> ResumeCommand {
        ResumeCommand {
            program: "codex".to_string(),
            args: vec![
                "resume".to_string(),
                id.to_string(),
                "-C".to_string(),
                working_dir.display().to_string(),
            ],
            cwd: working_dir.to_path_buf(),
        }
    }

    fn is_running(&self, _id: &str) -> bool {
        // Codex keeps no record of which sessions are open.
        false
    }

    fn resumes_by_directory(&self) -> bool {
        // `codex resume` finds a rollout by id from any directory.
        false
    }
}

/// The facts one rollout record gives.
fn read_rollout_record(facts: &mut Vec<Fact>, record: &Value) {
    if let Some(time) = record["timestamp"].as_str().and_then(parse_timestamp) {
        facts.push(Fact::Activity(time));
    }
    let payload = &record["payload"];
    match record["type"].as_str() {
        // A fork copies its parent's history, parent `session_meta` included,
        // after its own. The model keeps only the first identity and Git, so
        // only the first meta names this session.
        Some("session_meta") => {
            // A fork names the thread it copied as `forked_from_id`, which
            // alone doesn't make it a subagent.
            let spawn = &payload["source"]["subagent"];
            let kind = if !spawn.is_null() || !payload["parent_thread_id"].is_null() {
                ThreadKind::Subagent {
                    parent: non_empty(&payload["parent_thread_id"])
                        .or_else(|| non_empty(&spawn["thread_spawn"]["parent_thread_id"])),
                }
            } else {
                ThreadKind::Session
            };
            facts.push(Fact::Identity {
                id: payload["id"].as_str().map(str::to_string),
                kind,
            });
            facts.push(Fact::Git(RecordedGit {
                branch: non_empty(&payload["git"]["branch"]),
                repository_url: non_empty(&payload["git"]["repository_url"]),
            }));
            if let Some(cwd) = payload["cwd"].as_str() {
                facts.push(Fact::WorkingDir(PathBuf::from(cwd)));
            }
        }
        // `event_msg` records echo these messages, so only response items
        // are read.
        Some("response_item") => match payload["type"].as_str() {
            Some("message") => {
                let role = match payload["role"].as_str() {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => return,
                };
                for block in payload["content"].as_array().into_iter().flatten() {
                    let text = block["text"].as_str().unwrap_or("").trim();
                    if !(role == Role::User && is_injected(text)) {
                        facts.extend(Message::new(role, text).map(Fact::Message));
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
                facts.extend(Message::new(Role::ToolCall, &call).map(Fact::Message));
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let output = payload["output"].as_str().unwrap_or("");
                facts.extend(Message::new(Role::ToolOutput, output).map(Fact::Message));
            }
            _ => {}
        },
        _ => {}
    }
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
