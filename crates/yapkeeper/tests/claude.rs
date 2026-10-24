//! Drives `yapkeeper::list` and `search` over the Claude Code fixtures and
//! tempdir histories, and checks the sessions and search results its format
//! gives.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper::{Listing, RecordedGit, ResumeCommand, Roots, list};

mod listing;

use listing::{CLAUDE_FIXTURES, data_dir, found, found_with_tools, ids, listed, timestamp};

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const BUILD: &str = "22222222-2222-4222-8222-222222222222";
const RELEASE: &str = "33333333-3333-4333-8333-333333333333";
const ONLY_INJECTED: &str = "44444444-4444-4444-8444-444444444444";
const PROMPTLESS: &str = "55555555-5555-4555-8555-555555555555";
const NO_WORKING_DIR: &str = "66666666-6666-4666-8666-666666666666";
const NOTES: &str = "77777777-7777-4777-8777-777777777777";

/// `claude` as the Claude Code root, beside empty Codex and Pi roots under `tmp`.
fn roots(claude: &Path, tmp: &Path) -> Roots {
    Roots {
        claude: claude.to_path_buf(),
        codex: tmp.join("codex"),
        pi: tmp.join("pi"),
    }
}

/// Every session the fixtures give, read into a fresh index under `tmp`.
fn list_fixtures(tmp: &Path) -> Listing {
    list(&roots(Path::new(CLAUDE_FIXTURES), tmp), &data_dir(tmp)).unwrap()
}

/// Writes `contents` byte for byte as one session file under Claude Code root `root`.
fn write_session(root: &Path, id: &str, contents: &str) -> PathBuf {
    let project = root.join("projects").join("-synthetic-temp");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{id}.jsonl"));
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn last_active_is_the_latest_record_timestamp_or_else_the_file_mtime() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        listed(&list_fixtures(tmp.path()), LAUNCH)
            .session
            .last_active,
        timestamp("2026-10-20T10:05:00Z")
    );

    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let id = "aaaaaaaa-0000-4000-8000-000000000001";
    let path = write_session(
        &claude,
        id,
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

    let listing = list(&roots(&claude, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(listed(&listing, id).session.last_active, mtime);
}

#[test]
fn title_is_the_latest_custom_title_then_the_latest_ai_title_then_the_first_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    assert_eq!(listed(&listing, LAUNCH).session.title, "Launch checklist");
    assert_eq!(
        listed(&listing, BUILD).session.title,
        "Fix synthetic build failure"
    );
    assert_eq!(
        listed(&listing, NOTES).session.title,
        "Tidy the synthetic notes folder"
    );
}

#[test]
fn injected_context_is_never_a_title_or_searchable() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CLAUDE_FIXTURES), tmp.path());
    let listing = list_fixtures(tmp.path());
    assert_eq!(
        listed(&listing, RELEASE).session.title,
        "Summarise the synthetic release notes"
    );
    assert!(!ids(&listing).contains(&ONLY_INJECTED));
    // Both words appear in the fixtures only in injected records.
    assert_eq!(
        found_with_tools(&fixtures, tmp.path(), "Caveat"),
        [] as [&str; 0]
    );
    assert_eq!(
        found_with_tools(&fixtures, tmp.path(), "command-name"),
        [] as [&str; 0]
    );
    assert_eq!(found(&fixtures, tmp.path(), "Summarise"), [RELEASE]);
}

#[test]
fn sessions_without_a_prompt_or_a_working_directory_are_not_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    assert!(!ids(&listing).contains(&PROMPTLESS));
    assert!(!ids(&listing).contains(&NO_WORKING_DIR));
}

#[test]
fn a_malformed_line_and_an_unknown_record_type_do_not_stop_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CLAUDE_FIXTURES), tmp.path());
    let listing = list_fixtures(tmp.path());
    listed(&listing, NOTES);
    assert_eq!(listing.malformed_lines, 1);
    assert_eq!(found(&fixtures, tmp.path(), "tidy"), [NOTES]);
    // Only NOTES's unknown `future-record` has this word.
    assert_eq!(
        found_with_tools(&fixtures, tmp.path(), "something"),
        [] as [&str; 0]
    );
}

#[test]
fn an_unfinished_final_line_is_never_applied_or_counted_until_it_is_finished() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let id = "aaaaaaaa-0000-4000-8000-000000000006";
    let path = write_session(
        &claude,
        id,
        concat!(
            r#"{"type":"user","message":{"role":"user","content":"Finished prompt"},"cwd":"/synthetic/temp","timestamp":"2026-10-20T10:00:00Z"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"Unfinished title"}"#,
        ),
    );
    let roots = roots(&claude, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(listed(&listing, id).session.title, "Finished prompt");
    assert_eq!(listing.malformed_lines, 0);

    let mut file = fs::File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, b"\n").unwrap();
    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(listed(&listing, id).session.title, "Unfinished title");
}

#[test]
fn each_session_carries_the_command_that_resumes_it_in_its_working_directory() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        listed(&list_fixtures(tmp.path()), BUILD).session.resume,
        ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), BUILD.to_string()],
            cwd: PathBuf::from("/synthetic/beta"),
        }
    );
}

#[test]
fn working_dir_is_the_first_one_recorded_and_later_ones_are_kept_once() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    let launch = &listed(&listing, LAUNCH).session;
    assert_eq!(launch.working_dir, PathBuf::from("/synthetic/alpha"));
    assert_eq!(
        launch.other_working_dirs,
        [PathBuf::from("/synthetic/alpha/nested")]
    );
}

#[test]
fn subagent_transcripts_belong_to_their_session_instead_of_being_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CLAUDE_FIXTURES), tmp.path());
    let listing = list_fixtures(tmp.path());
    assert_eq!(ids(&listing).iter().filter(|id| **id == LAUNCH).count(), 1);
    assert!(!ids(&listing).iter().any(|id| id.contains("agent-")));
    assert_eq!(listed(&listing, LAUNCH).session.subagent_count, 2);
    let task = "Subagent task that must never be listed";
    assert_eq!(found(&fixtures, tmp.path(), task), [] as [&str; 0]);
    assert_eq!(found_with_tools(&fixtures, tmp.path(), task), [LAUNCH]);
}

#[test]
fn a_parent_uuid_chain_is_one_session_with_no_subagents() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CLAUDE_FIXTURES), tmp.path());
    let listing = list_fixtures(tmp.path());
    assert_eq!(ids(&listing).iter().filter(|id| **id == BUILD).count(), 1);
    assert_eq!(listed(&listing, BUILD).session.subagent_count, 0);
    // Only the reply at the end of BUILD's chain has this word.
    assert_eq!(found(&fixtures, tmp.path(), "missing"), [BUILD]);
}

#[test]
fn subagent_transcripts_whose_session_is_missing_are_left_out() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let kept = "aaaaaaaa-0000-4000-8000-000000000041";
    write_session(
        &claude,
        kept,
        concat!(
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            "\n"
        ),
    );
    let subagents =
        claude.join("projects/-synthetic-temp/aaaaaaaa-0000-4000-8000-000000000042/subagents");
    fs::create_dir_all(&subagents).unwrap();
    fs::write(
        subagents.join("agent-z.jsonl"),
        concat!(
            r#"{"type":"user","message":{"content":"An orphaned subagent task"},"cwd":"/synthetic/temp","isSidechain":true}"#,
            "\n"
        ),
    )
    .unwrap();
    let roots = roots(&claude, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [kept]);
    assert_eq!(listed(&listing, kept).session.subagent_count, 0);
    assert_eq!(
        found_with_tools(&roots, tmp.path(), "subagent"),
        [] as [&str; 0]
    );
    assert_eq!(found_with_tools(&roots, tmp.path(), "prompt"), [kept]);
}

#[test]
fn the_recap_is_the_latest_away_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let recapped = "aaaaaaaa-0000-4000-8000-000000000051";
    write_session(
        &claude,
        recapped,
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
    let plain = "aaaaaaaa-0000-4000-8000-000000000052";
    write_session(
        &claude,
        plain,
        concat!(
            r#"{"type":"user","message":{"content":"Another synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            "\n"
        ),
    );
    let roots = roots(&claude, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, recapped).session.recap.as_deref(),
        Some("The latest synthetic recap")
    );
    assert_eq!(listed(&listing, plain).session.recap, None);
    assert_eq!(found(&roots, tmp.path(), "latest"), [recapped]);
    assert_eq!(
        found_with_tools(&roots, tmp.path(), "earlier"),
        [] as [&str; 0]
    );
}

#[test]
fn linked_prs_come_from_pr_link_records_each_once() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let id = "aaaaaaaa-0000-4000-8000-000000000061";
    let pr_link = |number: u64| {
        format!(
            r#"{{"type":"pr-link","prNumber":{number},"prUrl":"https://example.com/acme/acme-api/pull/{number}","prRepository":"acme/acme-api"}}"#
        )
    };
    write_session(
        &claude,
        id,
        &format!(
            "{}\n{}\n{}\n{}\n",
            r#"{"type":"user","message":{"content":"A synthetic prompt"},"cwd":"/synthetic/temp"}"#,
            pr_link(42),
            pr_link(7),
            pr_link(42),
        ),
    );

    let listing = list(&roots(&claude, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(listed(&listing, id).session.linked_prs, [42, 7]);
}

#[test]
fn tool_calls_and_tool_output_are_tool_level() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CLAUDE_FIXTURES), tmp.path());
    // In the fixtures, only BUILD's `tool_use` input has the first word,
    // only its `tool_result` the second, and only its last reply the third.
    for tool_level in ["file_path", "output"] {
        assert_eq!(found(&fixtures, tmp.path(), tool_level), [] as [&str; 0]);
        assert_eq!(found_with_tools(&fixtures, tmp.path(), tool_level), [BUILD]);
    }
    assert_eq!(found(&fixtures, tmp.path(), "missing"), [BUILD]);
}

#[test]
fn text_past_the_cap_is_never_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let id = "aaaaaaaa-0000-4000-8000-000000000011";
    let prompt = format!("Opening {} closing", "a".repeat(4_000));
    write_session(
        &claude,
        id,
        &format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "message": { "content": prompt },
                "cwd": "/synthetic/temp",
            })
        ),
    );
    let roots = roots(&claude, tmp.path());

    assert_eq!(
        found_with_tools(&roots, tmp.path(), "closing"),
        [] as [&str; 0]
    );
    assert_eq!(found(&roots, tmp.path(), "Opening"), [id]);
}

#[test]
fn the_recorded_branch_is_the_first_git_branch_on_a_kept_record() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let id = "aaaaaaaa-0000-4000-8000-000000000031";
    write_session(
        &claude,
        id,
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

    let listing = list(&roots(&claude, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [id]);
    assert_eq!(
        listed(&listing, id).session.recorded_git,
        RecordedGit {
            branch: Some("feature/billing-retry".to_string()),
            repository_url: None,
        }
    );
}
