//! Drives the Codex adapter through `Codex::at(..).scan()` against committed
//! synthetic fixtures, and against files written into a tempdir when their
//! exact bytes matter.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use yapkeeper_codex::Codex;
use yapkeeper_model::{Adapter, Message, RecordedGit, ResumeCommand, Role, Scan, Session};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex");

const RETRY: &str = "aaaaaaaa-0000-7000-8000-00000000000a";
const ARCHIVED: &str = "bbbbbbbb-0000-7000-8000-00000000000b";
const FORK: &str = "cccccccc-0000-7000-8000-00000000000c";
const SUBAGENT: &str = "dddddddd-0000-7000-8000-00000000000d";

fn scan_fixtures() -> Scan {
    Codex::at(PathBuf::from(FIXTURES)).scan()
}

fn session<'a>(scan: &'a Scan, id: &str) -> &'a Session {
    scan.sessions
        .iter()
        .find(|session| session.id == id)
        .unwrap_or_else(|| panic!("session {id} is missing from {:?}", ids(scan)))
}

fn ids(scan: &Scan) -> Vec<&str> {
    let mut ids: Vec<&str> = scan
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    ids.sort_unstable();
    ids
}

fn timestamp(rfc3339: &str) -> SystemTime {
    SystemTime::from(rfc3339.parse::<jiff::Timestamp>().unwrap())
}

#[test]
fn active_and_archived_rollouts_and_forks_are_sessions_but_subagent_rollouts_are_not() {
    let scan = scan_fixtures();
    assert_eq!(ids(&scan), [RETRY, ARCHIVED, FORK]);
    assert!(!ids(&scan).contains(&SUBAGENT));
    assert!(
        scan.sessions
            .iter()
            .all(|session| session.provider == "codex")
    );
    assert_eq!(scan.malformed_lines, 0);
    assert_eq!(scan.searched, PathBuf::from(FIXTURES));
}

#[test]
fn a_subagent_rollout_belongs_to_its_parent_and_a_fork_is_no_subagent() {
    let scan = scan_fixtures();
    let retry = session(&scan, RETRY);
    let [subagent] = retry.subagents.as_slice() else {
        panic!("expected one subagent, got {:?}", retry.subagents);
    };
    assert_eq!(
        subagent.messages,
        [Message {
            role: Role::User,
            text: "Synthetic subagent task".to_string(),
        }]
    );
    assert_eq!(session(&scan, FORK).subagents, []);
}

#[test]
fn the_title_is_the_latest_thread_name_or_else_the_first_prompt() {
    let scan = scan_fixtures();
    assert_eq!(
        session(&scan, RETRY).title,
        "Tune the synthetic retry policy"
    );
    assert_eq!(
        session(&scan, ARCHIVED).title,
        "Archive the synthetic changelog"
    );
}

#[test]
fn messages_come_from_response_items_without_echoes_injected_context_or_tool_output() {
    let scan = scan_fixtures();
    let message = |role, text: &str| Message {
        role,
        text: text.to_string(),
    };
    assert_eq!(
        session(&scan, RETRY).messages,
        [
            message(Role::User, "Make the synthetic retry back off"),
            message(Role::ToolCall, r#"shell {"command":["ls"]}"#),
            message(Role::ToolCall, "apply_patch *** Begin Patch"),
            message(Role::Assistant, "The synthetic retry now backs off."),
        ]
    );
}

#[test]
fn a_fork_takes_its_identity_from_the_first_session_meta_and_keeps_later_directories() {
    let scan = scan_fixtures();
    let fork = session(&scan, FORK);
    assert_eq!(fork.title, "Fork of the retry policy");
    assert_eq!(fork.working_dir, PathBuf::from("/synthetic/epsilon"));
    assert_eq!(fork.other_working_dirs, [PathBuf::from("/synthetic/delta")]);
    assert_eq!(fork.last_active, timestamp("2026-10-21T10:00:00Z"));
}

#[test]
fn each_session_resumes_with_codex_resume_in_its_working_directory() {
    let scan = scan_fixtures();
    assert_eq!(
        session(&scan, RETRY).resume,
        ResumeCommand {
            program: "codex".to_string(),
            args: ["resume", RETRY, "-C", "/synthetic/delta"]
                .map(String::from)
                .to_vec(),
            cwd: PathBuf::from("/synthetic/delta"),
        }
    );
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
fn malformed_lines_are_counted_and_an_unfinished_final_line_is_left_for_later() {
    let home = tempfile::tempdir().unwrap();
    let unfinished = prompt("Still being written");
    write_rollout(
        home.path(),
        &format!(
            "{META}\nnot json\n{}\n{}",
            prompt("A finished prompt"),
            &unfinished[..unfinished.len() - 2]
        ),
    );
    fs::write(home.path().join("session_index.jsonl"), "not json either\n").unwrap();

    let scan = Codex::at(home.path().to_path_buf()).scan();
    assert_eq!(scan.malformed_lines, 2);
    let [session] = scan.sessions.as_slice() else {
        panic!("expected one session, got {:?}", ids(&scan));
    };
    assert_eq!(session.title, "A finished prompt");
    assert_eq!(session.messages.len(), 1);
}

#[test]
fn rollouts_without_a_prompt_or_a_session_meta_are_skipped() {
    let home = tempfile::tempdir().unwrap();
    write_rollout(
        home.path(),
        &format!(
            "{META}\n{}\n",
            prompt("<environment_context></environment_context>")
        ),
    );
    let day = home.path().join("sessions/2026/10/23");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout-2026-10-23T08-00-00-no-meta.jsonl"),
        format!("{}\n", prompt("A prompt with no session_meta")),
    )
    .unwrap();

    assert_eq!(Codex::at(home.path().to_path_buf()).scan().sessions, []);
}

#[test]
fn a_rollout_whose_first_session_meta_has_no_id_does_not_borrow_a_later_one() {
    let home = tempfile::tempdir().unwrap();
    let nameless = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"cwd":"/synthetic/eta","source":"cli"}}"#;
    write_rollout(
        home.path(),
        &format!("{nameless}\n{META}\n{}\n", prompt("A synthetic prompt")),
    );

    assert_eq!(Codex::at(home.path().to_path_buf()).scan().sessions, []);
}

#[test]
fn recorded_git_comes_from_the_first_session_meta_only() {
    let home = tempfile::tempdir().unwrap();
    let own = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"id":"eeeeeeee-0000-7000-8000-00000000000e","cwd":"/synthetic/eta","source":"cli","git":{"branch":"feature/billing-retry","repository_url":"git@example.com:acme/acme-api.git"}}}"#;
    let parents = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"id":"ffffffff-0000-7000-8000-00000000000f","cwd":"/synthetic/theta","source":"cli","git":{"branch":"main","repository_url":"git@example.com:acme/other.git"}}}"#;
    write_rollout(
        home.path(),
        &format!("{own}\n{parents}\n{}\n", prompt("A synthetic prompt")),
    );
    let scan = Codex::at(home.path().to_path_buf()).scan();
    let [session] = scan.sessions.as_slice() else {
        panic!("expected one session, got {:?}", ids(&scan));
    };
    assert_eq!(
        session.recorded_git,
        RecordedGit {
            branch: Some("feature/billing-retry".to_string()),
            repository_url: Some("git@example.com:acme/acme-api.git".to_string()),
        }
    );

    let home = tempfile::tempdir().unwrap();
    write_rollout(
        home.path(),
        &format!("{META}\n{}\n", prompt("A synthetic prompt")),
    );
    let scan = Codex::at(home.path().to_path_buf()).scan();
    let [session] = scan.sessions.as_slice() else {
        panic!("expected one session, got {:?}", ids(&scan));
    };
    assert_eq!(session.recorded_git, RecordedGit::default());
}

/// A rollout's first `session_meta`, with `extra` fields added to its payload.
fn meta(id: &str, extra: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/synthetic/eta","source":"cli"{extra}}}}}"#
    )
}

#[test]
fn a_subagent_of_a_subagent_belongs_to_the_top_session_and_one_without_a_session_is_left_out() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join("sessions/2026/10/22");
    fs::create_dir_all(&day).unwrap();
    let parent = "eeeeeeee-0000-7000-8000-000000000001";
    let child = "eeeeeeee-0000-7000-8000-000000000002";
    let grandchild = "eeeeeeee-0000-7000-8000-000000000003";
    let orphan = "eeeeeeee-0000-7000-8000-000000000004";
    let rollouts = [
        (parent, String::new(), "Parent prompt"),
        (
            child,
            format!(
                r#","parent_thread_id":"{parent}","source":{{"subagent":{{"other":"worker"}}}}"#
            ),
            "Child task",
        ),
        (
            grandchild,
            format!(
                r#","source":{{"subagent":{{"thread_spawn":{{"parent_thread_id":"{child}","depth":2}}}}}}"#
            ),
            "Grandchild task",
        ),
        (
            orphan,
            r#","parent_thread_id":"eeeeeeee-0000-7000-8000-0000000000ff""#.to_string(),
            "Orphaned subagent task",
        ),
    ];
    for (id, extra, text) in &rollouts {
        fs::write(
            day.join(format!("rollout-2026-10-22T08-00-00-{id}.jsonl")),
            format!("{}\n{}\n", meta(id, extra), prompt(text)),
        )
        .unwrap();
    }

    let scan = Codex::at(home.path().to_path_buf()).scan();
    assert_eq!(ids(&scan), [parent]);
    let mut texts: Vec<&str> = scan.sessions[0]
        .subagents
        .iter()
        .flat_map(|subagent| &subagent.messages)
        .map(|message| message.text.as_str())
        .collect();
    texts.sort_unstable();
    assert_eq!(texts, ["Child task", "Grandchild task"]);
}
