//! The Claude Code adapter. It owns where Claude Code keeps its history, the
//! JSONL grammar of a transcript and of a subagent transcript, which text
//! counts as a prompt, which records carry a title, a recap or a linked PR,
//! and how a session is resumed. The model folds what it reads into sessions.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Fact, Message, MissingHome, Reading, RecordedGit, ResumeCommand, Role, Sources,
    ThreadKind, TitleKind, jsonl_files, read_jsonl_from,
};

const PARSER_VERSION: u32 = 1;

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
    fn provider(&self) -> &'static str {
        "claude"
    }

    fn parser_version(&self) -> u32 {
        PARSER_VERSION
    }

    fn sources(&self) -> Sources {
        let projects = self.config_dir.join("projects");
        // A session's transcript is `<encoded-project>/<id>.jsonl`; the
        // agents it started are `<encoded-project>/<id>/subagents/*.jsonl`.
        let mut files = jsonl_files(&projects, 1);
        files.extend(
            jsonl_files(&projects, 3)
                .into_iter()
                .filter(|path| subagent_session(path).is_some()),
        );
        Sources {
            files,
            searched: projects,
        }
    }

    fn read(&self, source: &Path, from: u64) -> Reading {
        let mut facts = Vec::new();
        let mut malformed_lines = 0;
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        let offset = match subagent_session(source) {
            Some(session) => {
                if from == 0 {
                    facts.push(Fact::Identity {
                        id: stem.map(|stem| format!("{session}/{stem}")),
                        kind: ThreadKind::Subagent {
                            parent: Some(session),
                        },
                    });
                }
                // Every record in a subagent transcript is a sidechain, so
                // unlike a session's own records they are kept.
                read_jsonl_from(source, from, &mut malformed_lines, |record| {
                    let role = match record["type"].as_str() {
                        Some("user") => Role::User,
                        Some("assistant") => Role::Assistant,
                        _ => return,
                    };
                    if record["isMeta"].as_bool() != Some(true) {
                        push_content(&mut facts, role, &record["message"]["content"]);
                    }
                })
            }
            None => {
                if from == 0 {
                    facts.push(Fact::Identity {
                        id: stem,
                        kind: ThreadKind::Session,
                    });
                }
                read_jsonl_from(source, from, &mut malformed_lines, |record| {
                    read_transcript_record(&mut facts, &record);
                })
            }
        };
        Reading {
            facts,
            offset,
            malformed_lines,
        }
    }

    fn resume(&self, id: &str, working_dir: &Path) -> ResumeCommand {
        ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), id.to_string()],
            cwd: working_dir.to_path_buf(),
        }
    }
}

/// The session id whose `<id>/subagents/` folder holds `path`, or `None`
/// when `path` is a session's own transcript.
fn subagent_session(path: &Path) -> Option<String> {
    let folder = path.parent()?;
    if folder.file_name()? != "subagents" {
        return None;
    }
    Some(folder.parent()?.file_name()?.to_string_lossy().into_owned())
}

/// The facts one record of a session's own transcript gives.
fn read_transcript_record(facts: &mut Vec<Fact>, record: &Value) {
    if let Some(time) = record["timestamp"].as_str().and_then(parse_timestamp) {
        facts.push(Fact::Activity(time));
    }
    let role = match record["type"].as_str() {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        Some(kind @ ("custom-title" | "ai-title")) => {
            let (field, kind) = match kind {
                "custom-title" => ("customTitle", TitleKind::Named),
                _ => ("aiTitle", TitleKind::Generated),
            };
            if let Some(text) = non_empty(&record[field]) {
                facts.push(Fact::Title {
                    thread: None,
                    kind,
                    text,
                });
            }
            return;
        }
        Some("system") if record["subtype"] == "away_summary" => {
            facts.extend(non_empty(&record["content"]).map(Fact::Recap));
            return;
        }
        Some("pr-link") => {
            facts.extend(record["prNumber"].as_u64().map(Fact::LinkedPr));
            return;
        }
        _ => return,
    };
    if record["isMeta"].as_bool() == Some(true) || record["isSidechain"].as_bool() == Some(true) {
        return;
    }
    if let Some(cwd) = record["cwd"].as_str() {
        facts.push(Fact::WorkingDir(PathBuf::from(cwd)));
    }
    if let Some(branch) = non_empty(&record["gitBranch"]) {
        facts.push(Fact::Git(RecordedGit {
            branch: Some(branch),
            repository_url: None,
        }));
    }
    push_content(facts, role, &record["message"]["content"]);
}

/// A record's `message.content`, a string or an array of blocks, as messages.
/// Thinking and tool results are left out.
fn push_content(facts: &mut Vec<Fact>, role: Role, content: &Value) {
    match content {
        Value::String(text) => push_message(facts, role, text),
        Value::Array(blocks) => {
            for block in blocks {
                match block["type"].as_str() {
                    Some("text") => {
                        push_message(facts, role, block["text"].as_str().unwrap_or(""));
                    }
                    Some("tool_use") => {
                        let name = block["name"].as_str().unwrap_or("tool");
                        let call = format!("{name} {}", block["input"]);
                        push_message(facts, Role::ToolCall, &call);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn push_message(facts: &mut Vec<Fact>, role: Role, text: &str) {
    if role == Role::User && is_injected(text.trim()) {
        return;
    }
    facts.extend(Message::new(role, text).map(Fact::Message));
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
