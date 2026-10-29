//! The Claude Code adapter. It owns where Claude Code keeps its history, the
//! JSONL grammar of a transcript and of a subagent transcript, which text
//! counts as a prompt, which records carry a title, a recap or a linked PR,
//! how a session is resumed, and which sessions its running-session
//! metadata lists as open. The model folds what it reads into sessions.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use yapkeeper_model::{
    Adapter, Cost, Fact, Message, MissingHome, Reading, RecordedGit, ResumeCommand, Role,
    ThreadKind, TitleKind, Usage, jsonl_files, non_empty_text, read_jsonl_from, record_time,
};

const PARSER_VERSION: u32 = 3;

/// Dollars per million tokens of a model's input, output and cache reads.
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
}

/// List prices on the Claude API, by model id, which may also carry a date
/// such as `-20251101`. A model missing here has no known cost, so a new
/// model never borrows an older one's price. Claude Haiku 5.5 costs more on
/// prompts over 100K tokens, and some older models did on long prompts; the
/// standard rate is kept.
const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", price(10.0, 50.0, 0.25)),
    ("claude-mythos-5-1", price(10.0, 50.0, 0.25)),
    ("claude-fable-5", price(10.0, 50.0, 1.0)),
    ("claude-mythos-5", price(10.0, 50.0, 1.0)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.2)),
    ("claude-opus-5", price(5.0, 25.0, 0.5)),
    ("claude-opus-4-8", price(5.0, 25.0, 0.5)),
    ("claude-opus-4-7", price(5.0, 25.0, 0.5)),
    ("claude-opus-4-6", price(5.0, 25.0, 0.5)),
    ("claude-opus-4-5", price(5.0, 25.0, 0.5)),
    ("claude-opus-4-1", price(15.0, 75.0, 1.5)),
    ("claude-opus-4", price(15.0, 75.0, 1.5)),
    ("claude-sonnet-5-5", price(2.0, 10.0, 0.2)),
    ("claude-sonnet-5", price(2.0, 10.0, 0.2)),
    ("claude-sonnet-4-6", price(3.0, 15.0, 0.3)),
    ("claude-sonnet-4-5", price(3.0, 15.0, 0.3)),
    ("claude-sonnet-4", price(3.0, 15.0, 0.3)),
    ("claude-haiku-5-5", price(0.1, 0.5, 0.01)),
    ("claude-haiku-4-5", price(1.0, 5.0, 0.1)),
];

const fn price(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
    }
}

/// A running-session file is a few hundred bytes; anything past this is
/// not one, and is not read.
const MAX_RUNNING_FILE_BYTES: u64 = 64 * 1024;

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

    fn history_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }

    fn sources(&self) -> Vec<PathBuf> {
        let projects = self.history_dir();
        // A session's transcript is `<encoded-project>/<id>.jsonl`; the
        // agents it started are `<encoded-project>/<id>/subagents/*.jsonl`.
        let mut files = jsonl_files(&projects, 1);
        files.extend(
            jsonl_files(&projects, 3)
                .into_iter()
                .filter(|path| subagent_session(path).is_some()),
        );
        files
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
                    facts.extend(usage(&record));
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

    fn is_running(&self, id: &str) -> bool {
        // Claude Code writes `sessions/<pid>.json` for each open session.
        let Ok(entries) = fs::read_dir(self.config_dir.join("sessions")) else {
            return false;
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .any(|path| names_session(&path, id))
    }

    fn resumes_by_directory(&self) -> bool {
        // `--resume` looks under the project folder named after the
        // directory it starts in.
        true
    }
}

/// Whether the running-session file at `path` names session `id` as its
/// top-level `sessionId`; false for anything that isn't a readable JSON
/// file within `MAX_RUNNING_FILE_BYTES`.
fn names_session(path: &Path, id: &str) -> bool {
    // Checked before opening, as opening a FIFO would wait for a writer.
    if !fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return false;
    }
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_RUNNING_FILE_BYTES)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return false;
    }
    serde_json::from_slice::<Value>(&bytes)
        .is_ok_and(|record| record["sessionId"].as_str() == Some(id))
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
    if let Some(time) = record_time(record) {
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
            if let Some(text) = non_empty_text(&record[field]) {
                facts.push(Fact::Title {
                    thread: None,
                    kind,
                    text,
                });
            }
            return;
        }
        Some("system") if record["subtype"] == "away_summary" => {
            facts.extend(non_empty_text(&record["content"]).map(Fact::Recap));
            return;
        }
        Some("pr-link") => {
            facts.extend(record["prNumber"].as_u64().map(Fact::LinkedPr));
            return;
        }
        _ => return,
    };
    // A sidechain's responses were paid for all the same.
    facts.extend(usage(record));
    if record["isMeta"].as_bool() == Some(true) || record["isSidechain"].as_bool() == Some(true) {
        return;
    }
    if let Some(cwd) = record["cwd"].as_str() {
        facts.push(Fact::WorkingDir(PathBuf::from(cwd)));
    }
    if let Some(branch) = non_empty_text(&record["gitBranch"]) {
        facts.push(Fact::Git(RecordedGit {
            branch: Some(branch),
            repository_url: None,
        }));
    }
    push_content(facts, role, &record["message"]["content"]);
}

/// What the response an `assistant` record holds used, or `None` for any
/// other record. Claude Code writes one record per content block of a
/// response, each with the response's id and usage.
fn usage(record: &Value) -> Option<Fact> {
    if record["type"] != "assistant" {
        return None;
    }
    let message = &record["message"];
    let usage = &message["usage"];
    if !usage.is_object() {
        return None;
    }
    let count = |field: &str| usage[field].as_u64().unwrap_or(0);
    let (input, output) = (count("input_tokens"), count("output_tokens"));
    let (cache_read, cache_write) = (
        count("cache_read_input_tokens"),
        count("cache_creation_input_tokens"),
    );
    // Writes kept for an hour cost twice the input price, those kept for
    // five minutes a quarter more. Without the split, all are taken as the
    // cheaper kind.
    let hour_writes = usage["cache_creation"]["ephemeral_1h_input_tokens"]
        .as_u64()
        .unwrap_or(0)
        .min(cache_write);
    let cost = message["model"].as_str().and_then(price_of).map(|price| {
        let dollars = input as f64 * price.input
            + output as f64 * price.output
            + cache_read as f64 * price.cache_read
            + (cache_write - hour_writes) as f64 * price.input * 1.25
            + hour_writes as f64 * price.input * 2.0;
        Cost::from_dollars(dollars / 1e6)
    });
    Some(Fact::Usage(Usage {
        response: message["id"].as_str().map(str::to_string),
        tokens: input + output + cache_read + cache_write,
        cost,
    }))
}

/// The price of `model`: the entry of `PRICES` it names, alone or followed
/// by a date.
fn price_of(model: &str) -> Option<&'static Price> {
    PRICES
        .iter()
        .find(|(id, _)| match model.strip_prefix(id) {
            Some("") => true,
            Some(rest) => rest
                .strip_prefix('-')
                .is_some_and(|date| date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit())),
            None => false,
        })
        .map(|(_, price)| price)
}

/// A record's `message.content`, a string or an array of blocks, as messages.
/// Thinking is left out; tool results are tool output.
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
                    Some("tool_result") => {
                        let output = match &block["content"] {
                            Value::Array(parts) => parts
                                .iter()
                                .filter_map(|part| match part["type"].as_str() {
                                    Some("text") => part["text"].as_str(),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            content => content.as_str().unwrap_or("").to_string(),
                        };
                        push_message(facts, Role::ToolOutput, &output);
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

/// Claude Code's configuration directory for this process: `$CLAUDE_CONFIG_DIR`
/// when it is set and non-empty, otherwise `$HOME/.claude`.
pub fn config_dir() -> Result<PathBuf, MissingHome> {
    yapkeeper_model::root_from_env("CLAUDE_CONFIG_DIR", ".claude")
}
