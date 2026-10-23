//! Drives `yapkeeper::search` over synthetic Claude Code and Codex histories
//! written into a tempdir, and checks which sessions each query finds.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;
use yapkeeper::{DataDir, Listing, Query, Roots, rebuild, search};

/// A fresh `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and data directory, each empty.
struct Histories {
    dir: TempDir,
}

impl Histories {
    fn new() -> Histories {
        Histories {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn roots(&self) -> Roots {
        Roots {
            claude: self.dir.path().join("claude"),
            codex: self.dir.path().join("codex"),
        }
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn search(&self, query: Query) -> Listing {
        search(&self.roots(), &self.data(), &query).unwrap()
    }

    /// The ids of the sessions `text` finds, in listing order.
    fn found(&self, text: &str) -> Vec<String> {
        ids(&self.search(Query::parse(text, &self.roots().providers())))
    }

    /// The ids of the sessions `text` finds with tool-level text included.
    fn found_with_tools(&self, text: &str) -> Vec<String> {
        ids(&self.search(Query::parse(text, &self.roots().providers()).including_tools(true)))
    }

    /// Writes Claude Code session `id` as `records`, one per line.
    fn claude(&self, id: &str, records: &[Value]) -> PathBuf {
        let path = self
            .dir
            .path()
            .join("claude/projects/-synthetic-search")
            .join(format!("{id}.jsonl"));
        write_records(&path, records);
        path
    }

    /// Writes a subagent transcript of Claude Code session `session`.
    fn claude_subagent(&self, session: &str, agent: &str, records: &[Value]) {
        let path = self
            .dir
            .path()
            .join("claude/projects/-synthetic-search")
            .join(session)
            .join(format!("subagents/agent-{agent}.jsonl"));
        write_records(&path, records);
    }

    /// Writes Codex rollout `id`, its `session_meta` carrying `meta`'s fields,
    /// followed by `records` as `response_item` payloads.
    fn codex(&self, id: &str, meta: Value, records: &[Value]) {
        let path = self
            .dir
            .path()
            .join("codex/sessions/2026/10/22")
            .join(format!("rollout-2026-10-22T08-00-00-{id}.jsonl"));
        let mut payload = json!({ "id": id, "cwd": "/synthetic/search", "source": "cli" });
        payload
            .as_object_mut()
            .unwrap()
            .extend(meta.as_object().unwrap().clone());
        let mut lines = vec![json!({
            "timestamp": "2026-10-22T08:00:00Z",
            "type": "session_meta",
            "payload": payload,
        })];
        lines.extend(records.iter().map(|payload| {
            json!({
                "timestamp": "2026-10-22T08:00:01Z",
                "type": "response_item",
                "payload": payload,
            })
        }));
        write_records(&path, &lines);
    }
}

fn write_records(path: &Path, records: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lines: String = records.iter().map(|record| format!("{record}\n")).collect();
    fs::write(path, lines).unwrap();
}

fn ids(listing: &Listing) -> Vec<String> {
    listing
        .sessions
        .iter()
        .map(|listed| listed.session.id.clone())
        .collect()
}

/// A Claude Code record of `kind` (`user` or `assistant`) saying `content`.
fn said(kind: &str, content: Value) -> Value {
    json!({
        "type": kind,
        "message": { "content": content },
        "cwd": "/synthetic/search",
        "timestamp": "2026-10-20T10:00:00Z",
    })
}

fn prompt(text: &str) -> Value {
    said("user", json!(text))
}

fn reply(text: &str) -> Value {
    said("assistant", json!([{ "type": "text", "text": text }]))
}

/// A Claude Code record linking the session to pull request 42.
fn pr_link() -> Value {
    json!({ "type": "pr-link", "prNumber": 42, "timestamp": "2026-10-20T10:00:00Z" })
}

/// A Codex user message payload.
fn codex_prompt(text: &str) -> Value {
    json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] })
}

const A: &str = "aaaaaaaa-0000-4000-8000-000000000301";
const B: &str = "aaaaaaaa-0000-4000-8000-000000000302";
const C: &str = "aaaaaaaa-0000-4000-8000-000000000303";
const D: &str = "aaaaaaaa-0000-4000-8000-000000000304";

#[test]
fn every_word_must_match_somewhere_in_the_session() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[prompt("Why is this test flaky?"), reply("Add a retry.")],
    );
    histories.claude(
        B,
        &[
            prompt("Please retry the upload"),
            json!({ "type": "custom-title", "customTitle": "Flaky upload" }),
        ],
    );
    histories.claude(C, &[prompt("Another flaky test")]);
    histories.claude(D, &[prompt("Retry once more")]);

    let mut found = histories.found("flaky retry");
    found.sort();
    assert_eq!(found, [A, B]);
}

#[test]
fn one_and_two_character_words_match_like_longer_ones() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Port it to golang")]);
    histories.claude(B, &[prompt("Go further")]);
    histories.claude(C, &[prompt("Is this pr-42?")]);
    histories.claude(D, &[prompt("Nothing short here")]);

    let mut found = histories.found("go");
    found.sort();
    assert_eq!(found, [A, B]);
    assert_eq!(histories.found("G"), [B]);
    assert_eq!(histories.found("is pr"), [C]);
}

const CODEX: &str = "eeeeeeee-0000-7000-8000-000000000301";

#[test]
fn text_only_in_a_tool_call_is_found_only_with_tools() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[
            prompt("Read the log"),
            said(
                "assistant",
                json!([{ "type": "tool_use", "name": "Read", "input": { "file_path": "/synthetic/zebra.log" } }]),
            ),
        ],
    );
    histories.codex(
        CODEX,
        json!({}),
        &[
            codex_prompt("List the files"),
            json!({ "type": "function_call", "name": "shell", "arguments": "{\"command\":[\"zebra\"]}", "call_id": "c1" }),
        ],
    );

    assert!(histories.found("zebra").is_empty());
    let mut found = histories.found_with_tools("zebra");
    found.sort();
    assert_eq!(found, [A, CODEX]);
}

#[test]
fn text_only_in_tool_output_is_found_only_with_tools() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[
            prompt("Read the log"),
            said(
                "user",
                json!([{ "type": "tool_result", "tool_use_id": "t1", "content": "okapi sighted" }]),
            ),
        ],
    );
    histories.codex(
        CODEX,
        json!({}),
        &[
            codex_prompt("List the files"),
            json!({ "type": "function_call_output", "call_id": "c1", "output": "okapi.txt" }),
        ],
    );

    assert!(histories.found("okapi").is_empty());
    let mut found = histories.found_with_tools("okapi");
    found.sort();
    assert_eq!(found, [A, CODEX]);
}

const CODEX_SUBAGENT: &str = "eeeeeeee-0000-7000-8000-000000000302";

#[test]
fn text_only_in_a_subagent_transcript_finds_its_parent_session_only_with_tools() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release")]);
    histories.claude_subagent(A, "x", &[prompt("Look for a narwhal"), reply("No narwhal")]);
    histories.codex(CODEX, json!({}), &[codex_prompt("Plan the migration")]);
    histories.codex(
        CODEX_SUBAGENT,
        json!({ "source": { "subagent": { "thread_spawn": { "parent_thread_id": CODEX } } } }),
        &[codex_prompt("Find the narwhal")],
    );

    assert!(histories.found("narwhal").is_empty());
    let mut found = histories.found_with_tools("narwhal");
    found.sort();
    assert_eq!(found, [A, CODEX]);
}

#[test]
fn titles_and_recaps_are_searched() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[
            prompt("Plan the release"),
            json!({ "type": "custom-title", "customTitle": "Pelican launch" }),
        ],
    );
    histories.claude(
        B,
        &[
            prompt("Plan the migration"),
            json!({ "type": "system", "subtype": "away_summary", "content": "Moved the heron tables" }),
        ],
    );
    histories.codex(CODEX, json!({}), &[codex_prompt("Plan the rollout")]);
    fs::write(
        histories.dir.path().join("codex/session_index.jsonl"),
        format!(
            "{}\n",
            json!({ "id": CODEX, "thread_name": "Ibis rollout" })
        ),
    )
    .unwrap();

    assert_eq!(histories.found("pelican"), [A]);
    assert_eq!(histories.found("heron"), [B]);
    assert_eq!(histories.found("ibis"), [CODEX]);
}

#[test]
fn search_follows_files_that_grow_are_rewritten_or_disappear() {
    let histories = Histories::new();
    let path = histories.claude(A, &[prompt("Plan the release with care")]);
    histories.claude(B, &[prompt("Another plan")]);
    assert!(histories.found("wombat").is_empty());

    let grown = format!("{}\n", reply("A wombat appeared"));
    fs::File::options()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(grown.as_bytes())
        .unwrap();
    assert_eq!(histories.found("wombat"), [A]);

    // Shorter than what was read, so it is read again from the start.
    write_records(&path, &[prompt("Plan it")]);
    assert!(histories.found("wombat").is_empty());
    assert!(histories.found("care").is_empty());
    assert_eq!(histories.found("plan it"), [A]);

    fs::remove_file(&path).unwrap();
    assert!(histories.found("plan it").is_empty());
    assert_eq!(histories.found("plan"), [B]);
}

#[test]
fn words_that_are_full_text_syntax_match_literally() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[prompt(r#"Quote "retry then OR and NEAR, a* and col:x"#)],
    );
    histories.claude(B, &[prompt("Retry or near any column x")]);

    for word in [r#""retry"#, "OR", "NEAR", "a*", "col:x", r#"""#] {
        let listing = search(
            &histories.roots(),
            &histories.data(),
            &Query::parse(word, &histories.roots().providers()),
        );
        assert_eq!(ids(&listing.unwrap()), [A], "{word}");
    }
}

#[test]
fn case_is_ignored_and_matched_beyond_ascii() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Échec du build")]);
    histories.claude(B, &[prompt("Encore un échec")]);
    histories.claude(C, &[prompt("ÉCHEC TOTAL")]);

    let sorted = |text: &str| {
        let mut found = histories.found(text);
        found.sort();
        found
    };
    assert_eq!(sorted("échec"), [A, B, C]);
    assert_eq!(sorted("Échec"), [A]);
    assert_eq!(sorted("ÉCHEC"), [C]);
    assert_eq!(sorted("éch"), [A, B, C]);
}

#[test]
fn search_after_rebuild_finds_the_same_sessions() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release"), reply("Use a canary")]);
    histories.claude(B, &[prompt("Plan the migration")]);
    histories.codex(CODEX, json!({}), &[codex_prompt("A canary rollout")]);
    let before = histories.found("canary");
    assert_eq!(before.len(), 2);

    rebuild(&histories.roots(), &histories.data()).unwrap();
    assert_eq!(histories.found("canary"), before);
}

/// A Claude Code prompt recorded in `cwd`.
fn prompt_in(cwd: &str, text: &str) -> Value {
    prompt_at(cwd, "2026-10-20T10:00:00Z", text)
}

/// A Claude Code prompt recorded in `cwd` at `timestamp`.
fn prompt_at(cwd: &str, timestamp: &str, text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "content": text },
        "cwd": cwd,
        "timestamp": timestamp,
    })
}

const MAIN: &str = "eeeeeeee-0000-7000-8000-000000000311";
const WORKTREE: &str = "aaaaaaaa-0000-4000-8000-000000000311";
const SCRATCH: &str = "aaaaaaaa-0000-4000-8000-000000000312";

/// Orphaned sessions whose places are inferred: the `acme-api` main
/// checkout (from its recorded remote), its `billing-retry` worktree (from
/// the folder layout) and a `scratch/notes` folder.
fn places() -> Histories {
    let histories = Histories::new();
    histories.codex(
        MAIN,
        json!({
            "cwd": "/synthetic/acme-api",
            "git": { "repository_url": "git@example.com:acme/acme-api.git" },
        }),
        &[codex_prompt("Tidy the main checkout")],
    );
    histories.claude(
        WORKTREE,
        &[prompt_in(
            "/synthetic/acme-api.wt/billing-retry",
            "Retry the billing job",
        )],
    );
    histories.claude(
        SCRATCH,
        &[prompt_in("/synthetic/scratch/notes", "Jot some notes")],
    );
    histories
}

/// What `found` gives, sorted, for tests that don't check order.
fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[test]
fn at_a_project_finds_its_main_checkout_and_worktrees_and_at_a_worktree_only_that_one() {
    let histories = places();

    assert_eq!(sorted(histories.found("@acme-api")), [WORKTREE, MAIN]);
    assert_eq!(histories.found("@acme-api:billing-retry"), [WORKTREE]);
    assert_eq!(
        sorted(histories.found("@ACME-API")),
        sorted(histories.found("@acme-api"))
    );
}

#[test]
fn at_any_part_of_the_path_finds_a_folder_and_negated_leaves_it_out() {
    let histories = places();

    assert_eq!(histories.found("@scratch"), [SCRATCH]);
    assert_eq!(sorted(histories.found("!@scratch")), [WORKTREE, MAIN]);
}

#[test]
fn a_provider_filter_keeps_or_leaves_out_that_providers_sessions() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release")]);
    histories.claude(B, &[prompt("Ask codex for a second opinion")]);
    histories.codex(CODEX, json!({}), &[codex_prompt("Plan the rollout")]);

    assert_eq!(histories.found("#codex"), [CODEX]);
    assert_eq!(histories.found("#Codex"), [CODEX]);
    assert_eq!(sorted(histories.found("#claude")), [A, B]);
    for text in ["!codex", "!Codex", "not:codex"] {
        assert_eq!(sorted(histories.found(text)), [A, B], "{text}");
    }
    assert_eq!(histories.found("codex"), [B]);
}

#[test]
fn an_unknown_provider_or_state_finds_nothing_and_negated_excludes_nothing() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release")]);
    histories.codex(CODEX, json!({}), &[codex_prompt("Plan the rollout")]);

    for text in ["#pi", "is:bogus"] {
        assert!(histories.found(text).is_empty(), "{text}");
    }
    for text in ["!#pi", "!is:bogus"] {
        assert_eq!(sorted(histories.found(text)), [A, CODEX], "{text}");
    }
}

/// A live directory outside any temp folder: this crate's own.
const LIVE: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn is_orphaned_finds_sessions_whose_directory_is_gone() {
    let histories = Histories::new();
    histories.claude(A, &[prompt_in(LIVE, "Work that is still here")]);
    histories.claude(B, &[prompt_in("/synthetic/gone", "Work that is gone")]);

    assert_eq!(histories.found("is:orphaned"), [B]);
}

#[test]
fn is_temp_finds_sessions_in_a_system_temp_folder() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[prompt_in("/tmp/yapkeeper-synthetic-scratch", "Try it out")],
    );
    histories.claude(B, &[prompt_in("/synthetic/kept", "Keep this")]);

    assert_eq!(histories.found("is:temp"), [A]);
}

#[test]
fn is_git_finds_checkouts_and_worktrees_and_is_folder_the_rest() {
    let histories = places();

    assert_eq!(sorted(histories.found("is:git")), [WORKTREE, MAIN]);
    assert_eq!(histories.found("is:folder"), [SCRATCH]);
}

#[test]
fn is_pr_finds_sessions_with_a_linked_pull_request() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Open the pull request"), pr_link()]);
    histories.claude(B, &[prompt("No pull request here")]);

    assert_eq!(histories.found("is:pr"), [A]);
    assert_eq!(histories.found("IS:PR"), [A]);
}

#[test]
fn is_recap_finds_sessions_with_a_recap() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[
            prompt("Plan the migration"),
            json!({ "type": "system", "subtype": "away_summary", "content": "Moved the tables" }),
        ],
    );
    histories.claude(B, &[prompt("Plan the release")]);

    assert_eq!(histories.found("is:recap"), [A]);
}

#[test]
fn is_hidden_finds_nothing_until_sessions_can_be_hidden() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release")]);
    histories.codex(CODEX, json!({}), &[codex_prompt("Plan the rollout")]);

    assert!(histories.found("is:hidden").is_empty());
    assert_eq!(sorted(histories.found("!is:hidden")), [A, CODEX]);
}

#[test]
fn a_negated_state_or_word_leaves_out_the_sessions_it_names() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Open the pull request"), pr_link()]);
    histories.claude(B, &[prompt_in(LIVE, "Edit the temp settings")]);
    histories.claude(
        C,
        &[prompt_in("/tmp/yapkeeper-synthetic-scratch", "Try it out")],
    );
    histories.claude(D, &[prompt_in(LIVE, "Why is this test flaky?")]);

    // C is temp, so only queries with `is:temp` can find it.
    assert_eq!(sorted(histories.found("!is:pr")), [B, D]);
    assert_eq!(sorted(histories.found("is:temp !is:pr")), [C]);
    assert_eq!(sorted(histories.found("not:orphaned")), [B, D]);
    assert_eq!(sorted(histories.found("!temp")), [A, B, D]);
    assert!(histories.found("is:temp !temp").is_empty());
    assert_eq!(sorted(histories.found("!flaky")), [A, B]);
    assert_eq!(sorted(histories.found("is:temp !flaky")), [C]);
}

#[test]
fn a_negated_word_only_in_tool_output_excludes_only_with_tools() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Plan the release")]);
    histories.claude(
        B,
        &[
            prompt("Run the tests"),
            said(
                "user",
                json!([{ "type": "tool_result", "tool_use_id": "t1", "content": "1 flaky test" }]),
            ),
        ],
    );

    assert_eq!(sorted(histories.found("!flaky")), [A, B]);
    assert_eq!(histories.found_with_tools("!flaky"), [A]);
}

#[test]
fn is_and_pr_with_a_space_are_two_words_not_a_state() {
    let histories = Histories::new();
    histories.claude(A, &[prompt("Start this is a pr review")]);
    histories.claude(B, &[prompt("Open the pull request"), pr_link()]);

    assert_eq!(histories.found("is pr"), [A]);
}

#[test]
fn words_and_filters_must_all_match() {
    let histories = places();
    histories.codex(
        CODEX,
        json!({
            "cwd": "/synthetic/acme-api",
            "git": { "repository_url": "git@example.com:acme/acme-api.git" },
        }),
        &[codex_prompt("Retry the failed upload")],
    );
    histories.codex(
        CODEX_SUBAGENT,
        json!({ "cwd": "/synthetic/scratch/notes" }),
        &[codex_prompt("Retry somewhere else")],
    );

    // WORKTREE says retry in acme-api but is Claude Code; MAIN is Codex in
    // acme-api but never says retry.
    assert_eq!(histories.found("retry @acme-api #codex"), [CODEX]);
}

#[test]
fn temp_sessions_are_left_out_unless_the_query_asks_for_them() {
    let histories = Histories::new();
    histories.claude(
        A,
        &[prompt_in("/tmp/yapkeeper-synthetic-scratch", "Try it out")],
    );
    histories.claude(B, &[prompt_in("/synthetic/kept", "Try it for real")]);

    assert_eq!(histories.found(""), [B]);
    assert_eq!(histories.found("try"), [B]);
    assert_eq!(histories.found("is:temp"), [A]);
    assert_eq!(histories.found("try is:temp"), [A]);
}

#[test]
fn orphaned_sessions_come_after_the_rest_each_group_most_recent_first() {
    let histories = Histories::new();
    histories.claude(A, &[prompt_at(LIVE, "2026-10-01T10:00:00Z", "Oldest live")]);
    histories.claude(B, &[prompt_at(LIVE, "2026-10-02T10:00:00Z", "Newer live")]);
    histories.claude(
        C,
        &[prompt_at(
            "/synthetic/gone",
            "2026-10-20T10:00:00Z",
            "Newest gone",
        )],
    );
    histories.claude(
        D,
        &[prompt_at(
            "/synthetic/gone",
            "2026-10-10T10:00:00Z",
            "Older gone",
        )],
    );

    assert_eq!(histories.found(""), [B, A, C, D]);
}
