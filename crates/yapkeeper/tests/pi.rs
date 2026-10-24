//! Drives `yapkeeper::list` and `search` over the Pi fixtures, beside the
//! Claude Code and Codex ones, and checks the sessions and search results
//! Pi's format gives.

#![forbid(unsafe_code)]

use std::path::Path;

use yapkeeper::list;

mod listing;

use listing::{data_dir, fixture_roots, found, found_with_tools, ids, listed, timestamp};

const INBOX: &str = "01a00000-0000-7000-8000-00000000000a";
const NOTES: &str = "01b00000-0000-7000-8000-00000000000b";
const FORK: &str = "01f00000-0000-7000-8000-00000000000f";
const RUN: &str = "01c00000-0000-7000-8000-00000000000c";

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
