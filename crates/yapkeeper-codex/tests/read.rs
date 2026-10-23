//! Drives the Codex adapter through `Adapter::read`, the call the index
//! makes, against committed synthetic fixtures and files written into a
//! tempdir.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper_codex::Codex;
use yapkeeper_model::{Adapter, Fact, Message, Reading, Role, ThreadKind};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex");

const RETRY: &str = "aaaaaaaa-0000-7000-8000-00000000000a";
const ARCHIVED: &str = "bbbbbbbb-0000-7000-8000-00000000000b";
const SUBAGENT: &str = "dddddddd-0000-7000-8000-00000000000d";

/// Every fact in the fixture file at `relative`, read whole.
fn read_fixture(relative: &str) -> Reading {
    let home = PathBuf::from(FIXTURES);
    Codex::at(home.clone()).read(&home.join(relative), 0)
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

/// Writes `contents` byte for byte as one active rollout under `home`.
fn write_rollout(home: &Path, contents: &str) {
    let day = home.join("sessions/2026/10/22");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout-2026-10-22T08-00-00-synthetic.jsonl"),
        contents,
    )
    .unwrap();
}

const META: &str = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"id":"eeeeeeee-0000-7000-8000-00000000000e","cwd":"/synthetic/eta","source":"cli"}}"#;

fn prompt(text: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-10-22T08:00:01Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}}}"#
    )
}

#[test]
fn reading_on_from_the_offset_reached_gives_the_same_facts_as_one_whole_read() {
    let home = tempfile::tempdir().unwrap();
    let first = format!("{META}\n{}\n", prompt("A synthetic prompt"));
    let unfinished = prompt("A later prompt");
    write_rollout(home.path(), &format!("{first}{unfinished}"));
    let path = home
        .path()
        .join("sessions/2026/10/22/rollout-2026-10-22T08-00-00-synthetic.jsonl");
    let adapter = Codex::at(home.path().to_path_buf());

    let before = adapter.read(&path, 0);
    assert_eq!(before.offset, first.len() as u64);
    let mut file = fs::File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(
        &mut file,
        format!("\n{}\n", prompt("The last prompt")).as_bytes(),
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
fn a_rollout_gives_its_response_items_without_echoes_or_injected_context() {
    let reading = read_fixture(&format!(
        "sessions/2026/10/20/rollout-2026-10-20T11-00-00-{RETRY}.jsonl"
    ));
    assert_eq!(
        messages(reading),
        [
            message(Role::User, "Make the synthetic retry back off"),
            message(Role::ToolCall, r#"shell {"command":["ls"]}"#),
            message(Role::ToolOutput, "synthetic.txt"),
            message(Role::ToolCall, "apply_patch *** Begin Patch"),
            message(Role::ToolOutput, "Done"),
            message(Role::Assistant, "The synthetic retry now backs off."),
        ]
    );
}

/// The first `Identity` fact of `reading`.
fn identity(reading: Reading) -> Option<Fact> {
    reading
        .facts
        .into_iter()
        .find(|fact| matches!(fact, Fact::Identity { .. }))
}

#[test]
fn an_archived_rollout_is_a_session_and_a_subagent_rollout_names_its_parent() {
    let archived = read_fixture(&format!(
        "archived_sessions/rollout-2026-10-17T09-00-00-{ARCHIVED}.jsonl"
    ));
    assert_eq!(
        identity(archived),
        Some(Fact::Identity {
            id: Some(ARCHIVED.to_string()),
            kind: ThreadKind::Session,
        })
    );
    let subagent = read_fixture(&format!(
        "sessions/2026/10/21/rollout-2026-10-21T11-00-00-{SUBAGENT}.jsonl"
    ));
    assert_eq!(
        identity(subagent),
        Some(Fact::Identity {
            id: Some(SUBAGENT.to_string()),
            kind: ThreadKind::Subagent {
                parent: Some(RETRY.to_string()),
            },
        })
    );
}
