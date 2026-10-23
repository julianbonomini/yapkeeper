//! Drives the Claude Code adapter through `ClaudeCode::at(..).scan()` against
//! committed synthetic fixtures, and against files written into a tempdir
//! when their exact bytes or metadata matter.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use yapkeeper_claude::ClaudeCode;
use yapkeeper_model::{Adapter, Message, RecordedGit, ResumeCommand, Role, Scan, Session};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude");

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const BUILD: &str = "22222222-2222-4222-8222-222222222222";
const RELEASE: &str = "33333333-3333-4333-8333-333333333333";
const ONLY_INJECTED: &str = "44444444-4444-4444-8444-444444444444";
const PROMPTLESS: &str = "55555555-5555-4555-8555-555555555555";
const NO_WORKING_DIR: &str = "66666666-6666-4666-8666-666666666666";
const NOTES: &str = "77777777-7777-4777-8777-777777777777";

fn scan_fixtures() -> Scan {
    ClaudeCode::at(PathBuf::from(FIXTURES)).scan()
}

fn session<'a>(scan: &'a Scan, id: &str) -> &'a Session {
    scan.sessions
        .iter()
        .find(|session| session.id == id)
        .unwrap_or_else(|| panic!("session {id} is missing from {:?}", ids(scan)))
}

fn ids(scan: &Scan) -> Vec<&str> {
    scan.sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect()
}

/// Writes `contents` byte for byte as one session file under a fresh root.
fn write_session(root: &Path, id: &str, contents: &str) -> PathBuf {
    let project = root.join("projects").join("-synthetic-temp");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{id}.jsonl"));
    fs::write(&path, contents).unwrap();
    path
}

fn timestamp(rfc3339: &str) -> SystemTime {
    SystemTime::from(rfc3339.parse::<jiff::Timestamp>().unwrap())
}

#[test]
fn last_active_is_the_latest_record_timestamp_or_else_the_file_mtime() {
    let scan = scan_fixtures();
    assert_eq!(
        session(&scan, LAUNCH).last_active,
        timestamp("2026-10-20T10:05:00Z")
    );

    let root = tempfile::tempdir().unwrap();
    let path = write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000001",
        concat!(
            r#"{"type":"user","message":{"role":"user","content":"No timestamps here"},"cwd":"/synthetic/temp"}"#,
            "\n"
        ),
    );
    let mtime = timestamp("2026-01-02T03:04:05Z");
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    assert_eq!(
        session(&scan, "aaaaaaaa-0000-4000-8000-000000000001").last_active,
        mtime
    );
}

#[test]
fn title_is_the_latest_custom_title_then_the_latest_ai_title_then_the_first_prompt() {
    let scan = scan_fixtures();
    assert_eq!(session(&scan, LAUNCH).title, "Launch checklist");
    assert_eq!(session(&scan, BUILD).title, "Fix synthetic build failure");
    assert_eq!(
        session(&scan, NOTES).title,
        "Tidy the synthetic notes folder"
    );
}

#[test]
fn injected_context_is_never_a_title_or_a_message() {
    let scan = scan_fixtures();
    let release = session(&scan, RELEASE);
    assert_eq!(release.title, "Summarise the synthetic release notes");
    for message in &release.messages {
        for injected in ["Caveat:", "<command-name>", "# AGENTS.md", "Meta text"] {
            assert!(
                !message.text.contains(injected),
                "{injected:?} leaked into {message:?}"
            );
        }
    }
    assert!(!ids(&scan).contains(&ONLY_INJECTED));
}

#[test]
fn sessions_without_a_prompt_or_a_working_directory_are_skipped() {
    let scan = scan_fixtures();
    assert!(!ids(&scan).contains(&PROMPTLESS));
    assert!(!ids(&scan).contains(&NO_WORKING_DIR));
}

#[test]
fn a_malformed_line_and_an_unknown_record_type_do_not_stop_discovery() {
    let scan = scan_fixtures();
    let notes = session(&scan, NOTES);
    let texts: Vec<&str> = notes
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "Tidy the synthetic notes folder",
            "The notes folder is tidy."
        ]
    );
    assert_eq!(scan.malformed_lines, 1);
}

#[test]
fn an_unfinished_final_line_is_never_applied_or_counted() {
    let root = tempfile::tempdir().unwrap();
    let id = "aaaaaaaa-0000-4000-8000-000000000006";
    write_session(
        root.path(),
        id,
        concat!(
            r#"{"type":"user","message":{"role":"user","content":"Finished prompt"},"cwd":"/synthetic/temp","timestamp":"2026-10-20T10:00:00Z"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"Unfinished title"}"#,
        ),
    );

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    assert_eq!(session(&scan, id).title, "Finished prompt");
    assert_eq!(scan.malformed_lines, 0);
}

#[test]
fn each_session_carries_the_command_that_resumes_it_in_its_working_directory() {
    let scan = scan_fixtures();
    assert_eq!(
        session(&scan, BUILD).resume,
        ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), BUILD.to_string()],
            cwd: PathBuf::from("/synthetic/beta"),
        }
    );
}

#[test]
fn working_dir_is_the_first_one_recorded_and_later_ones_are_kept_once() {
    let scan = scan_fixtures();
    let launch = session(&scan, LAUNCH);
    assert_eq!(launch.working_dir, PathBuf::from("/synthetic/alpha"));
    assert_eq!(
        launch.other_working_dirs,
        [PathBuf::from("/synthetic/alpha/nested")]
    );
}

#[test]
fn subagent_transcripts_belong_to_their_session_instead_of_being_listed() {
    let scan = scan_fixtures();
    assert_eq!(ids(&scan).iter().filter(|id| **id == LAUNCH).count(), 1);
    assert!(!ids(&scan).iter().any(|id| id.starts_with("agent-")));
    let launch = session(&scan, LAUNCH);
    assert_eq!(launch.subagents.len(), 2);
    let texts: Vec<&str> = launch
        .subagents
        .iter()
        .flat_map(|subagent| &subagent.messages)
        .map(|message| message.text.as_str())
        .collect();
    assert!(texts.contains(&"Subagent task that must never be listed"));
    assert!(texts.contains(&"Second synthetic subagent reply"));
    assert!(
        launch
            .messages
            .iter()
            .all(|message| !message.text.contains("ubagent"))
    );
}

#[test]
fn a_parent_uuid_chain_is_one_session_with_no_subagents() {
    let scan = scan_fixtures();
    let build = session(&scan, BUILD);
    assert_eq!(build.messages.len(), 4);
    assert_eq!(build.subagents, []);
}

#[test]
fn subagent_transcripts_whose_session_is_missing_are_left_out() {
    let root = tempfile::tempdir().unwrap();
    let kept = "aaaaaaaa-0000-4000-8000-000000000041";
    write_session(
        root.path(),
        kept,
        concat!(
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            "\n"
        ),
    );
    let subagents = root
        .path()
        .join("projects/-synthetic-temp/aaaaaaaa-0000-4000-8000-000000000042/subagents");
    fs::create_dir_all(&subagents).unwrap();
    fs::write(
        subagents.join("agent-z.jsonl"),
        concat!(
            r#"{"type":"user","message":{"content":"An orphaned subagent task"},"cwd":"/synthetic/temp","isSidechain":true}"#,
            "\n"
        ),
    )
    .unwrap();

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    assert_eq!(ids(&scan), [kept]);
    assert_eq!(scan.sessions[0].subagents, []);
}

#[test]
fn the_recap_is_the_latest_away_summary() {
    let root = tempfile::tempdir().unwrap();
    write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000051",
        concat!(
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            "\n",
            r#"{"type":"system","subtype":"away_summary","content":"An earlier synthetic recap","isMeta":false}"#,
            "\n",
            r#"{"type":"system","subtype":"away_summary","content":"The latest synthetic recap","isMeta":false}"#,
            "\n",
            r#"{"type":"system","subtype":"turn_duration","content":"Not a recap"}"#,
            "\n",
        ),
    );
    write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000052",
        concat!(
            r#"{"type":"user","message":{"content":"Another synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            "\n"
        ),
    );

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    assert_eq!(
        session(&scan, "aaaaaaaa-0000-4000-8000-000000000051")
            .recap
            .as_deref(),
        Some("The latest synthetic recap")
    );
    assert_eq!(
        session(&scan, "aaaaaaaa-0000-4000-8000-000000000052").recap,
        None
    );
}

#[test]
fn linked_prs_come_from_pr_link_records_each_once() {
    let root = tempfile::tempdir().unwrap();
    let id = "aaaaaaaa-0000-4000-8000-000000000061";
    let pr_link = |number: u64| {
        format!(
            r#"{{"type":"pr-link","prNumber":{number},"prUrl":"https://example.com/acme/acme-api/pull/{number}","prRepository":"acme/acme-api"}}"#
        )
    };
    write_session(
        root.path(),
        id,
        &format!(
            "{}\n{}\n{}\n{}\n",
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            pr_link(42),
            pr_link(7),
            pr_link(42),
        ),
    );

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    assert_eq!(session(&scan, id).linked_prs, [42, 7]);
}

#[test]
fn tool_calls_are_kept_apart_from_ordinary_messages() {
    let scan = scan_fixtures();
    assert_eq!(
        session(&scan, BUILD).messages,
        [
            Message {
                role: Role::User,
                text: "Why does the synthetic build fail?".to_string(),
            },
            Message {
                role: Role::Assistant,
                text: "Let me read the build log.".to_string(),
            },
            Message {
                role: Role::ToolCall,
                text: r#"Read {"file_path":"/synthetic/beta/build.log"}"#.to_string(),
            },
            Message {
                role: Role::Assistant,
                text: "The build fails because a file is missing.".to_string(),
            },
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
    let id = "aaaaaaaa-0000-4000-8000-000000000011";
    write_session(root.path(), id, &contents);

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    let messages = &session(&scan, id).messages;
    assert_eq!(messages[0].text, format!("{}…", "a".repeat(3_999)));
    assert_eq!(
        messages[1].text,
        format!(r#"Bash {{"command":"{}…"#, "b".repeat(282))
    );
}

#[test]
fn the_recorded_branch_is_the_first_git_branch_on_a_kept_record() {
    let root = tempfile::tempdir().unwrap();
    write_session(
        root.path(),
        "aaaaaaaa-0000-4000-8000-000000000031",
        concat!(
            r#"{"type":"user","isMeta":true,"message":{"content":"Injected"},"cwd":"/synthetic/temp","gitBranch":"meta-branch"}"#,
            "\n",
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp","gitBranch":""}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":"Done"},"cwd":"/synthetic/temp","gitBranch":"feature/billing-retry"}"#,
            "\n",
            r#"{"type":"user","message":{"content":"Again"},"cwd":"/synthetic/temp","gitBranch":"later"}"#,
            "\n",
        ),
    );

    let scan = ClaudeCode::at(root.path().to_path_buf()).scan();
    let [session] = scan.sessions.as_slice() else {
        panic!("expected one session, got {:?}", ids(&scan));
    };
    assert_eq!(
        session.recorded_git,
        RecordedGit {
            branch: Some("feature/billing-retry".to_string()),
            repository_url: None,
        }
    );
}
