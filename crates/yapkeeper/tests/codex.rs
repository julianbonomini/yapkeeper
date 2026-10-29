//! Drives `yapkeeper::list` and `search` over the Codex fixtures and tempdir
//! histories, and checks the sessions and search results its format gives.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper::{Listing, RecordedGit, ResumeCommand, Roots, Spent, list};

mod listing;

use listing::{CODEX_FIXTURES, data_dir, found, found_with_tools, ids, listed, timestamp};

const RETRY: &str = "aaaaaaaa-0000-7000-8000-00000000000a";
const ARCHIVED: &str = "bbbbbbbb-0000-7000-8000-00000000000b";
const FORK: &str = "cccccccc-0000-7000-8000-00000000000c";
const SUBAGENT: &str = "dddddddd-0000-7000-8000-00000000000d";

/// `codex` as the Codex root, beside empty Claude Code and Pi roots under `tmp`.
fn roots(codex: &Path, tmp: &Path) -> Roots {
    Roots {
        claude: tmp.join("claude"),
        codex: codex.to_path_buf(),
        pi: tmp.join("pi"),
    }
}

/// Every session the fixtures give, read into a fresh index under `tmp`.
fn list_fixtures(tmp: &Path) -> Listing {
    list(&roots(Path::new(CODEX_FIXTURES), tmp), &data_dir(tmp)).unwrap()
}

#[test]
fn active_and_archived_rollouts_and_forks_are_listed_but_subagent_rollouts_are_not() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    let mut listed_ids = ids(&listing);
    listed_ids.sort_unstable();
    assert_eq!(listed_ids, [RETRY, ARCHIVED, FORK]);
    assert!(!listed_ids.contains(&SUBAGENT));
    assert!(
        listing
            .sessions
            .iter()
            .all(|listed| listed.session.provider == "codex")
    );
    assert_eq!(listing.malformed_lines, 0);
}

#[test]
fn a_subagent_rollout_belongs_to_its_parent_and_a_fork_is_no_subagent() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CODEX_FIXTURES), tmp.path());
    let listing = list_fixtures(tmp.path());
    assert_eq!(listed(&listing, RETRY).session.subagent_count, 1);
    assert_eq!(listed(&listing, FORK).session.subagent_count, 0);
    let task = "Synthetic subagent task";
    assert_eq!(found(&fixtures, tmp.path(), task), [] as [&str; 0]);
    assert_eq!(found_with_tools(&fixtures, tmp.path(), task), [RETRY]);
}

#[test]
fn the_title_is_the_latest_thread_name_or_else_the_first_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    assert_eq!(
        listed(&listing, RETRY).session.title,
        "Tune the synthetic retry policy"
    );
    assert_eq!(
        listed(&listing, ARCHIVED).session.title,
        "Archive the synthetic changelog"
    );
}

/// Writes `contents` byte for byte as one active rollout under Codex home `home`.
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
const META_ID: &str = "eeeeeeee-0000-7000-8000-00000000000e";

fn prompt(text: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-10-22T08:00:01Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}}}"#
    )
}

#[test]
fn echoes_and_injected_context_are_never_searched() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CODEX_FIXTURES), tmp.path());
    // In the fixtures, each word appears only in RETRY's developer message,
    // its `AGENTS.md` block or its environment block.
    for injected in ["developer", "rules", "environment_context"] {
        assert_eq!(
            found_with_tools(&fixtures, tmp.path(), injected),
            [] as [&str; 0]
        );
    }
    assert_eq!(found(&fixtures, tmp.path(), "backs"), [RETRY]);

    let codex = tmp.path().join("codex");
    write_rollout(
        &codex,
        &format!(
            "{META}\n{}\n{}\n{}\n",
            prompt("A synthetic prompt"),
            r#"{"timestamp":"2026-10-22T08:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"An echoed prompt"}}"#,
            r#"{"timestamp":"2026-10-22T08:00:03Z","type":"event_msg","payload":{"type":"agent_message","message":"An echoed reply"}}"#,
        ),
    );
    let roots = roots(&codex, tmp.path());
    assert_eq!(
        found_with_tools(&roots, tmp.path(), "echoed"),
        [] as [&str; 0]
    );
    assert_eq!(found(&roots, tmp.path(), "prompt"), [META_ID]);
}

#[test]
fn tool_calls_and_tool_output_are_tool_level() {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = roots(Path::new(CODEX_FIXTURES), tmp.path());
    // In the fixtures, only RETRY's `function_call` has the first word and
    // only its `function_call_output` the second.
    for tool_level in ["shell", "txt"] {
        assert_eq!(found(&fixtures, tmp.path(), tool_level), [] as [&str; 0]);
        assert_eq!(found_with_tools(&fixtures, tmp.path(), tool_level), [RETRY]);
    }
}

#[test]
fn a_fork_takes_its_identity_from_the_first_session_meta_and_keeps_later_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list_fixtures(tmp.path());
    let fork = &listed(&listing, FORK).session;
    assert_eq!(fork.title, "Fork of the retry policy");
    assert_eq!(fork.working_dir, PathBuf::from("/synthetic/epsilon"));
    assert_eq!(fork.other_working_dirs, [PathBuf::from("/synthetic/delta")]);
    assert_eq!(fork.last_active, timestamp("2026-10-21T10:00:00Z"));
}

#[test]
fn each_session_resumes_with_codex_resume_in_its_working_directory() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        listed(&list_fixtures(tmp.path()), RETRY).session.resume,
        ResumeCommand {
            program: "codex".to_string(),
            args: ["resume", RETRY, "-C", "/synthetic/delta"]
                .map(String::from)
                .to_vec(),
            cwd: PathBuf::from("/synthetic/delta"),
        }
    );
}

#[test]
fn malformed_lines_are_counted_and_an_unfinished_final_line_is_left_for_later() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    let unfinished = prompt("Still being written");
    let (written, rest) = unfinished.split_at(unfinished.len() - 2);
    write_rollout(
        &codex,
        &format!(
            "{META}\nnot json\n{}\n{written}",
            prompt("A finished prompt")
        ),
    );
    fs::write(codex.join("session_index.jsonl"), "not json either\n").unwrap();
    let roots = roots(&codex, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(listing.malformed_lines, 2);
    assert_eq!(ids(&listing), [META_ID]);
    assert_eq!(listed(&listing, META_ID).session.title, "A finished prompt");
    assert_eq!(found(&roots, tmp.path(), "Still"), [] as [&str; 0]);

    let path = codex.join("sessions/2026/10/22/rollout-2026-10-22T08-00-00-synthetic.jsonl");
    let mut file = fs::File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, format!("{rest}\n").as_bytes()).unwrap();
    assert_eq!(found(&roots, tmp.path(), "Still"), [META_ID]);
}

#[test]
fn rollouts_without_a_prompt_or_a_session_meta_are_not_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    write_rollout(
        &codex,
        &format!(
            "{META}\n{}\n",
            prompt("<environment_context></environment_context>")
        ),
    );
    let day = codex.join("sessions/2026/10/23");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout-2026-10-23T08-00-00-no-meta.jsonl"),
        format!("{}\n", prompt("A prompt with no session_meta")),
    )
    .unwrap();

    let listing = list(&roots(&codex, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [] as [&str; 0]);
}

#[test]
fn a_rollout_whose_first_session_meta_has_no_id_does_not_borrow_a_later_one() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    let nameless = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"cwd":"/synthetic/eta","source":"cli"}}"#;
    write_rollout(
        &codex,
        &format!("{nameless}\n{META}\n{}\n", prompt("A synthetic prompt")),
    );

    let listing = list(&roots(&codex, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [] as [&str; 0]);
}

#[test]
fn recorded_git_comes_from_the_first_session_meta_only() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    let own = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"id":"eeeeeeee-0000-7000-8000-00000000000e","cwd":"/synthetic/eta","source":"cli","git":{"branch":"feature/billing-retry","repository_url":"git@example.com:acme/acme-api.git"}}}"#;
    let parents = r#"{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{"id":"ffffffff-0000-7000-8000-00000000000f","cwd":"/synthetic/theta","source":"cli","git":{"branch":"main","repository_url":"git@example.com:acme/other.git"}}}"#;
    write_rollout(
        &codex,
        &format!("{own}\n{parents}\n{}\n", prompt("A synthetic prompt")),
    );
    let listing = list(&roots(&codex, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, META_ID).session.recorded_git,
        RecordedGit {
            branch: Some("feature/billing-retry".to_string()),
            repository_url: Some("git@example.com:acme/acme-api.git".to_string()),
        }
    );

    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    write_rollout(
        &codex,
        &format!("{META}\n{parents}\n{}\n", prompt("A synthetic prompt")),
    );
    let listing = list(&roots(&codex, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, META_ID).session.recorded_git,
        RecordedGit::default()
    );
}

/// A rollout's first `session_meta`, with `extra` fields added to its payload.
fn meta(id: &str, extra: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/synthetic/eta","source":"cli"{extra}}}}}"#
    )
}

#[test]
fn a_subagent_of_a_subagent_belongs_to_the_top_session_and_one_without_a_session_is_left_out() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tmp.path().join("codex");
    let day = codex.join("sessions/2026/10/22");
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
    let roots = roots(&codex, tmp.path());

    let listing = list(&roots, &data_dir(tmp.path())).unwrap();
    assert_eq!(ids(&listing), [parent]);
    assert_eq!(listed(&listing, parent).session.subagent_count, 2);
    for subagent in ["Child", "Grandchild"] {
        assert_eq!(found(&roots, tmp.path(), subagent), [] as [&str; 0]);
        assert_eq!(found_with_tools(&roots, tmp.path(), subagent), [parent]);
    }
    // Only the orphan's prompt has this word.
    assert_eq!(
        found_with_tools(&roots, tmp.path(), "subagent"),
        [] as [&str; 0]
    );
}

#[test]
fn a_session_spends_its_latest_running_total_with_no_known_cost() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("codex");
    let total = |tokens: u64| {
        format!(
            r#"{{"timestamp":"2026-10-22T08:00:02Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{tokens},"output_tokens":0,"total_tokens":{tokens}}}}}}}}}"#
        )
    };
    let no_info = r#"{"timestamp":"2026-10-22T08:00:02Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#;
    write_rollout(
        &home,
        &format!(
            "{META}\n{}\n{}\n{}\n{no_info}\n{}\n",
            prompt("A synthetic prompt"),
            total(100),
            total(100),
            total(250),
        ),
    );

    let listing = list(&roots(&home, tmp.path()), &data_dir(tmp.path())).unwrap();
    assert_eq!(
        listed(&listing, META_ID).session.spent,
        Spent {
            tokens: 250,
            cost: None,
        }
    );
}
