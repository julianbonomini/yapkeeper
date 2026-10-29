//! Drives `yapkeeper::list` and `search` over the Pi fixtures, beside the
//! Claude Code and Codex ones, and checks the sessions and search results
//! Pi's format gives.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper::{Cost, Roots, Spent, list};

mod listing;

use listing::{data_dir, fixture_roots, found, found_with_tools, ids, listed, timestamp};

const INBOX: &str = "01a00000-0000-7000-8000-00000000000a";
const NOTES: &str = "01b00000-0000-7000-8000-00000000000b";
const FORK: &str = "01f00000-0000-7000-8000-00000000000f";
const RUN: &str = "01c00000-0000-7000-8000-00000000000c";

const SESSION_FILE: &str = "2026-10-22T08-00-00-000Z_01e00000-0000-7000-8000-00000000000e";

/// Only `pi` as Pi's root; the other providers' roots don't exist.
fn pi_roots(pi: &Path, tmp: &Path) -> Roots {
    Roots {
        claude: tmp.join("no-claude"),
        codex: tmp.join("no-codex"),
        pi: pi.to_path_buf(),
    }
}

/// Writes a Pi session file at `path` with header `id` and one prompt.
fn write_pi_session(path: &Path, id: &str, prompt: &str) {
    write_pi_file(
        path,
        &format!(r#"{{"type":"session","version":3,"id":"{id}","cwd":"/synthetic/pi-eta"}}"#),
        prompt,
    );
}

/// Writes a Pi file at `path` with the session `header` and one prompt.
fn write_pi_file(path: &Path, header: &str, prompt: &str) {
    let message = format!(
        r#"{{"type":"message","id":"m1","parentId":null,"timestamp":"2026-10-22T08:00:01.000Z","message":{{"role":"user","content":[{{"type":"text","text":"{prompt}"}}]}}}}"#
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{header}\n{message}\n")).unwrap();
}

/// The folder Pi keeps `SESSION_FILE`'s runs and forks in, under `pi`.
fn session_folder(pi: &Path) -> PathBuf {
    pi.join("--synthetic-pi-eta--").join(SESSION_FILE)
}

#[test]
fn a_pi_session_was_last_active_at_its_latest_record_and_resumes_where_it_ran() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list(&fixture_roots(), &data_dir(tmp.path())).unwrap();
    let inbox = &listed(&listing, INBOX).session;
    assert_eq!(inbox.provider, "pi");
    assert_eq!(inbox.working_dir, Path::new("/synthetic/pi-alpha"));
    assert_eq!(inbox.last_active, timestamp("2026-10-21T09:00:09Z"));
    assert_eq!(inbox.resume.program, "pi");
    assert_eq!(inbox.resume.args, ["--session", INBOX]);
    assert_eq!(inbox.resume.cwd, Path::new("/synthetic/pi-alpha"));
}

#[test]
fn hash_pi_finds_only_top_level_pi_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let mut pi = found(&fixture_roots(), tmp.path(), "#pi");
    pi.sort_unstable();
    assert_eq!(pi, [INBOX, NOTES]);
    let listing = list(&fixture_roots(), &data_dir(tmp.path())).unwrap();
    for nested in [FORK, RUN] {
        assert!(!ids(&listing).contains(&nested), "{nested} is listed");
    }
}

#[test]
fn the_title_is_the_latest_session_name_or_else_the_first_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list(&fixture_roots(), &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, INBOX).session.title,
        "Sort the synthetic inbox by sender"
    );
    assert_eq!(
        listed(&listing, NOTES).session.title,
        "Draft the synthetic release notes"
    );
}

#[test]
fn tool_text_is_found_only_with_tools_and_thinking_never() {
    let tmp = tempfile::tempdir().unwrap();
    // In the fixtures, `zircon` appears only in a tool result and
    // `Pondering` only in a thinking block.
    assert_eq!(
        found(&fixture_roots(), tmp.path(), "#pi zircon"),
        [] as [&str; 0]
    );
    assert_eq!(
        found_with_tools(&fixture_roots(), tmp.path(), "#pi zircon"),
        [INBOX]
    );
    assert_eq!(
        found_with_tools(&fixture_roots(), tmp.path(), "#pi Pondering"),
        [] as [&str; 0]
    );
}

#[test]
fn subagent_runs_belong_to_their_session_and_are_searched_only_as_tool_text() {
    let tmp = tempfile::tempdir().unwrap();
    let pi = tmp.path().join("pi");
    let session = "01e00000-0000-7000-8000-00000000000e";
    let folder = session_folder(&pi);
    write_pi_session(
        &folder.with_extension("jsonl"),
        session,
        "A synthetic prompt",
    );
    for (run, id) in [
        ("run-a", "01e00000-0000-7000-8000-0000000000e1"),
        ("run-b", "01e00000-0000-7000-8000-0000000000e2"),
    ] {
        write_pi_session(
            &folder.join(run).join("run-0/session.jsonl"),
            id,
            "A synthetic subagent task",
        );
    }
    let roots = pi_roots(&pi, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [session]);
    assert_eq!(listed(&listing, session).session.subagent_count, 2);
    assert_eq!(found(&roots, tmp.path(), "subagent"), [] as [&str; 0]);
    assert_eq!(found_with_tools(&roots, tmp.path(), "subagent"), [session]);
}

#[test]
fn a_fork_is_a_subagent_of_the_session_whose_folder_holds_it() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list(&fixture_roots(), &data_dir(tmp.path())).unwrap();
    // The fixture session has one run and one fork.
    assert_eq!(listed(&listing, INBOX).session.subagent_count, 2);
    assert!(!ids(&listing).contains(&FORK), "the fork is listed");
    assert_eq!(
        found(&fixture_roots(), tmp.path(), "#pi fork"),
        [] as [&str; 0]
    );
    assert_eq!(
        found_with_tools(&fixture_roots(), tmp.path(), "#pi fork"),
        [INBOX]
    );
}

#[test]
fn a_parent_session_link_alone_does_not_make_a_subagent() {
    let tmp = tempfile::tempdir().unwrap();
    let pi = tmp.path().join("pi");
    let session = "01e00000-0000-7000-8000-00000000000e";
    let folder = session_folder(&pi);
    let parent_file = folder.with_extension("jsonl");
    write_pi_session(&parent_file, session, "A synthetic prompt");
    // Beside its parent's file, a file naming a `parentSession` is a session.
    let linked = "01e00000-0000-7000-8000-0000000000e3";
    write_pi_file(
        &pi.join(format!(
            "--synthetic-pi-eta--/2026-10-22T09-00-00-000Z_{linked}.jsonl"
        )),
        &format!(
            r#"{{"type":"session","version":3,"id":"{linked}","cwd":"/synthetic/pi-eta","parentSession":"{}"}}"#,
            parent_file.display()
        ),
        "A synthetic linked prompt",
    );
    let roots = pi_roots(&pi, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    let mut listed_ids = ids(&listing);
    listed_ids.sort_unstable();
    assert_eq!(listed_ids, [session, linked]);
    assert_eq!(listed(&listing, session).session.subagent_count, 0);
}

#[test]
fn a_run_whose_session_file_is_missing_is_left_out() {
    let tmp = tempfile::tempdir().unwrap();
    let pi = tmp.path().join("pi");
    let kept = "01d00000-0000-7000-8000-00000000000d";
    write_pi_session(
        &pi.join("--synthetic-pi-eta--/2026-10-22T09-00-00-000Z_01d00000-0000-7000-8000-00000000000d.jsonl"),
        kept,
        "A synthetic prompt",
    );
    write_pi_session(
        &session_folder(&pi).join("run-a/run-0/session.jsonl"),
        "01e00000-0000-7000-8000-0000000000e1",
        "An orphaned subagent task",
    );
    let roots = pi_roots(&pi, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [kept]);
    assert_eq!(listed(&listing, kept).session.subagent_count, 0);
    assert_eq!(
        found_with_tools(&roots, tmp.path(), "subagent"),
        [] as [&str; 0]
    );
}

/// A Pi `assistant` message written at `timestamp` that used `tokens` and
/// cost `cost` dollars.
fn pi_response(timestamp: &str, tokens: u64, cost: f64) -> String {
    format!(
        r#"{{"type":"message","id":"r-{timestamp}","parentId":"m1","timestamp":"{timestamp}","message":{{"role":"assistant","content":[{{"type":"text","text":"A synthetic reply"}}],"model":"synthetic","usage":{{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":{tokens},"cost":{{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":{cost}}}}}}}}}"#
    )
}

#[test]
fn a_session_spends_what_pi_recorded_and_a_fork_only_what_it_added() {
    let tmp = tempfile::tempdir().unwrap();
    let pi = tmp.path().join("pi");
    let session = "01e00000-0000-7000-8000-00000000000e";
    let folder = session_folder(&pi);
    let session_file = folder.with_extension("jsonl");
    write_pi_session(&session_file, session, "A synthetic prompt");
    let copied = pi_response("2026-10-22T08:00:02.000Z", 300, 0.012);
    fs::write(
        &session_file,
        format!("{}{copied}\n", fs::read_to_string(&session_file).unwrap()),
    )
    .unwrap();
    // Started an hour after the session, beginning with a copy of it.
    let fork = "01e00000-0000-7000-8000-0000000000e4";
    let fork_file = folder
        .join("forks")
        .join(format!("2026-10-22T09-00-00-000Z_{fork}.jsonl"));
    write_pi_file(
        &fork_file,
        &format!(
            r#"{{"type":"session","version":3,"id":"{fork}","cwd":"/synthetic/pi-eta","parentSession":"{}"}}"#,
            session_file.display()
        ),
        "A synthetic subagent task",
    );
    let added = pi_response("2026-10-22T09:00:05.000Z", 1000, 0.5);
    fs::write(
        &fork_file,
        format!(
            "{}{copied}\n{added}\n",
            fs::read_to_string(&fork_file).unwrap()
        ),
    )
    .unwrap();

    let listing = list(&pi_roots(&pi, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, session).session.spent,
        Spent {
            tokens: 1300,
            cost: Some(Cost::from_micros(512_000)),
        }
    );
}
