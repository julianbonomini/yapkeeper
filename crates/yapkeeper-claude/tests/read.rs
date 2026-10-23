//! Drives the Claude Code adapter through `Adapter::read`, the call the
//! index makes, against committed synthetic fixtures and files written into
//! a tempdir.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper_claude::ClaudeCode;
use yapkeeper_model::{Adapter, Fact, Message, Reading, Role, ThreadKind};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude");

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const BUILD: &str = "22222222-2222-4222-8222-222222222222";
const RELEASE: &str = "33333333-3333-4333-8333-333333333333";

/// Every fact in the fixture file at `relative`, read whole.
fn read_fixture(relative: &str) -> Reading {
    let root = PathBuf::from(FIXTURES);
    ClaudeCode::at(root.clone()).read(&root.join(relative), 0)
}

/// The `Message` facts of `reading`, in file order.
fn messages(reading: Reading) -> Vec<Message> {
    reading
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            Fact::Message(message) => Some(message),
            _ => None,
        })
        .collect()
}

fn message(role: Role, text: &str) -> Message {
    Message {
        role,
        text: text.to_string(),
    }
}

/// Writes `contents` byte for byte as one session file under a fresh root.
fn write_session(root: &Path, id: &str, contents: &str) -> PathBuf {
    let project = root.join("projects").join("-synthetic-temp");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{id}.jsonl"));
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn reading_on_from_the_offset_reached_gives_the_same_facts_as_one_whole_read() {
    let root = tempfile::tempdir().unwrap();
    let first = concat!(
        r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp","timestamp":"2026-10-20T10:00:00Z"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":"A synthetic reply"},"cwd":"/synthetic/temp","timestamp":"2026-10-20T10:01:00Z"}"#,
        "\n",
    );
    let unfinished = r#"{"type":"custom-title","customTitle":"A synthetic title"}"#;
    let path = write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000071",
        &format!("{first}{unfinished}"),
    );
    let adapter = ClaudeCode::at(root.path().to_path_buf());

    let before = adapter.read(&path, 0);
    assert_eq!(before.offset, first.len() as u64);
    let mut file = fs::File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(
        &mut file,
        concat!("\n", r#"{"type":"pr-link","prNumber":42}"#, "\n").as_bytes(),
    )
    .unwrap();
    let after = adapter.read(&path, before.offset);

    let whole = adapter.read(&path, 0);
    let mut resumed = before.facts;
    resumed.extend(after.facts);
    assert_eq!(resumed, whole.facts);
    assert_eq!(after.offset, whole.offset);
}

#[test]
fn a_transcript_gives_its_messages_with_tool_calls_and_output_kept_apart() {
    assert_eq!(
        messages(read_fixture(&format!(
            "projects/-synthetic-beta/{BUILD}.jsonl"
        ))),
        [
            message(Role::User, "Why does the synthetic build fail?"),
            message(Role::Assistant, "Let me read the build log."),
            message(
                Role::ToolCall,
                r#"Read {"file_path":"/synthetic/beta/build.log"}"#
            ),
            message(Role::ToolOutput, "Synthetic tool output."),
            message(
                Role::Assistant,
                "The build fails because a file is missing."
            ),
        ]
    );
}

#[test]
fn tool_results_with_string_or_array_content_are_tool_output() {
    let record = |content: serde_json::Value| {
        serde_json::json!({
            "type": "user",
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": content }] },
            "cwd": "/synthetic/temp",
        })
        .to_string()
    };
    let contents = format!(
        "{}\n{}\n{}\n",
        r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
        record(serde_json::json!("<synthetic> output that looks injected")),
        record(serde_json::json!([
            { "type": "text", "text": "line one" },
            { "type": "image" },
            { "type": "text", "text": "line two" },
        ])),
    );
    let root = tempfile::tempdir().unwrap();
    let path = write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000012",
        &contents,
    );

    let reading = ClaudeCode::at(root.path().to_path_buf()).read(&path, 0);
    assert_eq!(
        messages(reading),
        [
            message(Role::User, "A synthetic prompt"),
            message(Role::ToolOutput, "<synthetic> output that looks injected"),
            message(Role::ToolOutput, "line one\nline two"),
        ]
    );
}

#[test]
fn long_messages_and_tool_calls_are_capped_on_a_char_boundary() {
    // Each 'é' is two bytes and starts one byte before the cap, so a cut at
    // the cap itself would split it.
    let prompt = format!("{}é and more", "a".repeat(3_999));
    let command = format!("{}é and more", "b".repeat(282));
    let record = |content: serde_json::Value| {
        serde_json::json!({
            "type": if content.is_string() { "user" } else { "assistant" },
            "message": { "content": content },
            "cwd": "/synthetic/temp",
        })
        .to_string()
    };
    let contents = format!(
        "{}\n{}\n",
        record(serde_json::json!(prompt)),
        record(serde_json::json!([
            { "type": "tool_use", "name": "Bash", "input": { "command": command } }
        ])),
    );
    let root = tempfile::tempdir().unwrap();
    let path = write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000011",
        &contents,
    );

    let reading = ClaudeCode::at(root.path().to_path_buf()).read(&path, 0);
    assert_eq!(
        messages(reading),
        [
            message(Role::User, &format!("{}…", "a".repeat(3_999))),
            message(
                Role::ToolCall,
                &format!(r#"Bash {{"command":"{}…"#, "b".repeat(282))
            ),
        ]
    );
}

#[test]
fn injected_context_gives_no_message() {
    let reading = read_fixture(&format!("projects/-synthetic-beta/{RELEASE}.jsonl"));
    for message in messages(reading) {
        for injected in ["Caveat:", "<command-name>", "# AGENTS.md", "Meta text"] {
            assert!(
                !message.text.contains(injected),
                "{injected:?} leaked into {message:?}"
            );
        }
    }
}

#[test]
fn a_subagent_transcript_names_its_session_as_its_parent() {
    let reading = read_fixture(&format!(
        "projects/-synthetic-alpha/{LAUNCH}/subagents/agent-x.jsonl"
    ));
    assert_eq!(
        reading.facts.first(),
        Some(&Fact::Identity {
            id: Some(format!("{LAUNCH}/agent-x")),
            kind: ThreadKind::Subagent {
                parent: Some(LAUNCH.to_string()),
            },
        })
    );
}
