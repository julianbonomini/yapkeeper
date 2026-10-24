//! Drives the Pi adapter through `Adapter::read`, the call the index makes,
//! and `Adapter::resume`, against committed synthetic fixtures and files
//! written into a tempdir.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper_model::{
    Adapter, Fact, Message, Reading, ResumeCommand, Role, ThreadKind, TitleKind,
};
use yapkeeper_pi::Pi;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pi");

const INBOX: &str = "01a00000-0000-7000-8000-00000000000a";

const HEADER: &str = r#"{"type":"session","version":3,"id":"01e00000-0000-7000-8000-00000000000e","timestamp":"2026-10-22T08:00:00.000Z","cwd":"/synthetic/pi-eta"}"#;

fn prompt(text: &str) -> String {
    format!(
        r#"{{"type":"message","id":"e1","parentId":null,"timestamp":"2026-10-22T08:00:01.000Z","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

/// Writes `contents` byte for byte as one session file under a fresh root,
/// and reads it whole.
fn read_written(contents: &str) -> Reading {
    let root = tempfile::tempdir().unwrap();
    let path = write_session(root.path(), contents);
    Pi::at(root.path().to_path_buf()).read(&path, 0)
}

fn write_session(root: &Path, contents: &str) -> PathBuf {
    let folder = root.join("--synthetic-pi-eta--");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join("2026-10-22T08-00-00-000Z_synthetic.jsonl");
    fs::write(&path, contents).unwrap();
    path
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

#[test]
fn a_session_file_gives_its_header_names_and_messages_without_thinking() {
    let root = PathBuf::from(FIXTURES);
    let reading = Pi::at(root.clone()).read(
        &root.join(format!(
            "--synthetic-pi-alpha--/2026-10-21T09-00-00-000Z_{INBOX}.jsonl"
        )),
        0,
    );
    assert_eq!(reading.malformed_lines, 0);
    assert!(reading.facts.contains(&Fact::Identity {
        id: Some(INBOX.to_string()),
        kind: ThreadKind::Session,
    }));
    assert!(
        reading
            .facts
            .contains(&Fact::WorkingDir(PathBuf::from("/synthetic/pi-alpha")))
    );
    let names: Vec<&Fact> = reading
        .facts
        .iter()
        .filter(|fact| matches!(fact, Fact::Title { .. }))
        .collect();
    assert_eq!(
        names,
        ["First synthetic name", "Sort the synthetic inbox by sender"]
            .map(|text| Fact::Title {
                thread: None,
                kind: TitleKind::Named,
                text: text.to_string(),
            })
            .iter()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        messages(reading),
        [
            message(Role::User, "Sort the synthetic inbox"),
            message(Role::Assistant, "Reading the synthetic inbox."),
            message(Role::ToolCall, r#"read {"path":"inbox.txt"}"#),
            message(Role::ToolOutput, "zircon invoice"),
            message(Role::ToolCall, "ls inbox"),
            message(Role::ToolOutput, "inbox.txt"),
            message(Role::ToolOutput, "Background task finished"),
            message(Role::Assistant, "The synthetic inbox is sorted."),
        ]
    );
}

#[test]
fn a_malformed_line_is_counted_and_the_lines_around_it_are_read() {
    let reading = read_written(&format!(
        "{HEADER}\n{{\"type\":\"message\",\n{}\n",
        prompt("After the malformed line")
    ));
    assert_eq!(reading.malformed_lines, 1);
    assert_eq!(
        messages(reading),
        [message(Role::User, "After the malformed line")]
    );
}

#[test]
fn an_unfinished_final_line_is_left_for_the_next_read() {
    let first = format!("{HEADER}\n");
    let reading = read_written(&format!("{first}{}", prompt("Still being written")));
    assert_eq!(reading.offset, first.len() as u64);
    assert_eq!(reading.malformed_lines, 0);
    assert_eq!(messages(reading), []);
}

#[test]
fn a_record_without_a_type_is_skipped() {
    let untyped =
        r#"{"id":"e2","message":{"role":"user","content":[{"type":"text","text":"Untyped"}]}}"#;
    let reading = read_written(&format!("{HEADER}\n{untyped}\n{}\n", prompt("After it")));
    assert_eq!(reading.malformed_lines, 0);
    assert_eq!(messages(reading), [message(Role::User, "After it")]);
}

#[test]
fn a_record_of_an_unknown_type_is_skipped() {
    let unknown = r#"{"type":"synthetic_future","id":"e2","message":{"role":"user","content":[{"type":"text","text":"Unknown"}]}}"#;
    let reading = read_written(&format!("{HEADER}\n{unknown}\n{}\n", prompt("After it")));
    assert_eq!(reading.malformed_lines, 0);
    assert_eq!(messages(reading), [message(Role::User, "After it")]);
}

#[test]
fn a_session_resumes_with_pi_session_in_its_recorded_directory() {
    let pi = Pi::at(PathBuf::from(FIXTURES));
    assert_eq!(
        pi.resume(INBOX, Path::new("/synthetic/pi-alpha")),
        ResumeCommand {
            program: "pi".to_string(),
            args: vec!["--session".to_string(), INBOX.to_string()],
            cwd: PathBuf::from("/synthetic/pi-alpha"),
        }
    );
    assert!(pi.resumes_by_directory());
}
