//! Drives `yapkeeper::list` and `yapkeeper::rebuild` over synthetic histories
//! that change between calls, and checks what the index read and returned.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use yapkeeper::{
    Availability, DataDir, Listing, Location, Query, Roots, Session, hide, list, rebuild, search,
};

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
            pi: self.dir.path().join("pi"),
        }
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn list(&self) -> Listing {
        list(&self.roots(), &self.data()).unwrap()
    }

    /// The transcript of Claude Code session `id`, which may not exist yet.
    fn claude_transcript(&self, id: &str) -> PathBuf {
        self.dir
            .path()
            .join("claude/projects/-synthetic-index")
            .join(format!("{id}.jsonl"))
    }

    /// Writes a Claude Code transcript with one prompt made in `cwd`.
    fn claude_session(&self, id: &str, prompt: &str, cwd: &str) -> PathBuf {
        let path = self.claude_transcript(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, claude_prompt(prompt, cwd)).unwrap();
        path
    }

    /// Writes a Codex rollout for session `id` with one prompt made in `cwd`.
    fn codex_session(&self, id: &str, prompt: &str, cwd: &str) -> PathBuf {
        let day = self.dir.path().join("codex/sessions/2026/10/22");
        fs::create_dir_all(&day).unwrap();
        let path = day.join(format!("rollout-2026-10-22T08-00-00-{id}.jsonl"));
        let meta = format!(
            r#"{{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","source":"cli"}}}}"#
        );
        let prompt = format!(
            r#"{{"timestamp":"2026-10-22T08:00:01Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{prompt}"}}]}}}}"#
        );
        fs::write(&path, format!("{meta}\n{prompt}\n")).unwrap();
        path
    }
}

/// One complete Claude Code user record.
fn claude_prompt(prompt: &str, cwd: &str) -> String {
    format!(
        r#"{{"type":"user","message":{{"content":"{prompt}"}},"cwd":"{cwd}","timestamp":"2026-10-20T10:00:00Z"}}"#
    ) + "\n"
}

fn append(path: &Path, text: &str) {
    fs::File::options()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

/// The title of the session listed with `id`.
fn title<'a>(listing: &'a Listing, id: &str) -> &'a str {
    listing
        .sessions
        .iter()
        .find(|listed| listed.session.id == id)
        .map(|listed| listed.session.title.as_str())
        .unwrap_or_else(|| panic!("session {id} is not listed"))
}

fn sessions(listing: &Listing) -> Vec<Session> {
    listing
        .sessions
        .iter()
        .map(|listed| listed.session.clone())
        .collect()
}

#[test]
fn a_second_list_with_no_changes_reads_no_history_file() {
    let histories = Histories::new();
    histories.claude_session(
        "aaaaaaaa-0000-4000-8000-000000000101",
        "A synthetic prompt",
        "/synthetic/one",
    );
    histories.codex_session(
        "eeeeeeee-0000-7000-8000-000000000101",
        "Another synthetic prompt",
        "/synthetic/two",
    );

    let first = histories.list();
    assert_eq!(first.sessions.len(), 2);
    assert_eq!(first.files_read, 2);

    let second = histories.list();
    assert_eq!(sessions(&second), sessions(&first));
    assert_eq!(second.files_read, 0);
    assert_eq!(second.bytes_read, 0);
}

#[test]
fn appending_a_title_changes_it_after_reading_only_the_new_bytes() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000102";
    let path = histories.claude_session(id, "A synthetic prompt", "/synthetic/one");
    assert_eq!(title(&histories.list(), id), "A synthetic prompt");

    let line = concat!(
        r#"{"type":"custom-title","customTitle":"A synthetic title"}"#,
        "\n"
    );
    append(&path, line);
    let listing = histories.list();
    assert_eq!(title(&listing, id), "A synthetic title");
    assert_eq!(listing.files_read, 1);
    assert_eq!(listing.bytes_read, line.len() as u64);
}

#[test]
fn adding_a_rollout_makes_its_session_appear() {
    let histories = Histories::new();
    histories.claude_session(
        "aaaaaaaa-0000-4000-8000-000000000103",
        "A synthetic prompt",
        "/synthetic/one",
    );
    assert_eq!(histories.list().sessions.len(), 1);

    let id = "eeeeeeee-0000-7000-8000-000000000103";
    histories.codex_session(id, "A new synthetic prompt", "/synthetic/two");
    let listing = histories.list();
    assert_eq!(listing.sessions.len(), 2);
    assert_eq!(title(&listing, id), "A new synthetic prompt");
    assert_eq!(listing.files_read, 1);
}

/// The session listed with `id`.
fn session<'a>(listing: &'a Listing, id: &str) -> &'a Session {
    listing
        .sessions
        .iter()
        .map(|listed| &listed.session)
        .find(|session| session.id == id)
        .unwrap_or_else(|| panic!("session {id} is not listed"))
}

#[test]
fn a_truncated_transcript_is_read_again_from_the_start() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000104";
    let path = histories.claude_session(id, "The first synthetic prompt", "/synthetic/one");
    append(
        &path,
        concat!(
            r#"{"type":"custom-title","customTitle":"An old synthetic title"}"#,
            "\n"
        ),
    );
    histories.list();

    fs::write(&path, claude_prompt("A short prompt", "/synthetic/two")).unwrap();
    let listing = histories.list();
    let session = session(&listing, id);
    assert_eq!(session.title, "A short prompt");
    assert_eq!(session.working_dir, PathBuf::from("/synthetic/two"));
    assert_eq!(session.other_working_dirs, Vec::<PathBuf>::new());
}

#[test]
fn a_new_file_renamed_over_a_transcript_is_read_again_from_the_start() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000105";
    let path = histories.claude_session(id, "An old synthetic prompt", "/synthetic/one");
    append(
        &path,
        concat!(
            r#"{"type":"custom-title","customTitle":"An old synthetic title"}"#,
            "\n"
        ),
    );
    histories.list();

    let replacement = path.with_extension("tmp");
    let prompt = "A replacement prompt, longer than everything it replaces";
    fs::write(
        &replacement,
        claude_prompt(prompt, "/synthetic/two") + &claude_prompt(prompt, "/synthetic/two"),
    )
    .unwrap();
    assert!(fs::metadata(&replacement).unwrap().len() > fs::metadata(&path).unwrap().len());
    fs::rename(&replacement, &path).unwrap();

    let listing = histories.list();
    let session = session(&listing, id);
    assert_eq!(session.title, prompt);
    assert_eq!(session.working_dir, PathBuf::from("/synthetic/two"));
}

#[test]
fn deleting_a_history_file_removes_what_it_recorded() {
    let histories = Histories::new();
    let kept = "aaaaaaaa-0000-4000-8000-000000000106";
    let deleted = "aaaaaaaa-0000-4000-8000-000000000107";
    histories.claude_session(kept, "A kept prompt", "/synthetic/one");
    let transcript = histories.claude_session(deleted, "A deleted prompt", "/synthetic/one");
    let subagents = histories
        .claude_transcript(kept)
        .with_extension("")
        .join("subagents");
    fs::create_dir_all(&subagents).unwrap();
    for agent in ["agent-x", "agent-y"] {
        fs::write(
            subagents.join(format!("{agent}.jsonl")),
            claude_prompt("A synthetic subagent task", "/synthetic/one"),
        )
        .unwrap();
    }
    let codex = "eeeeeeee-0000-7000-8000-000000000106";
    histories.codex_session(codex, "A synthetic Codex prompt", "/synthetic/two");
    let session_index = histories.dir.path().join("codex/session_index.jsonl");
    fs::write(
        &session_index,
        format!(r#"{{"id":"{codex}","thread_name":"A synthetic Codex name"}}"#) + "\n",
    )
    .unwrap();
    let listing = histories.list();
    assert_eq!(listing.sessions.len(), 3);
    assert_eq!(session(&listing, kept).subagent_count, 2);
    assert_eq!(title(&listing, codex), "A synthetic Codex name");

    fs::remove_file(&transcript).unwrap();
    fs::remove_file(subagents.join("agent-y.jsonl")).unwrap();
    fs::remove_file(&session_index).unwrap();
    let listing = histories.list();
    assert_eq!(listing.sessions.len(), 2);
    assert!(
        listing
            .sessions
            .iter()
            .all(|listed| listed.session.id != deleted)
    );
    assert_eq!(session(&listing, kept).subagent_count, 1);
    assert_eq!(title(&listing, codex), "A synthetic Codex prompt");
}

#[test]
fn an_unfinished_final_line_is_indexed_only_once_it_is_finished() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000108";
    let path = histories.claude_session(id, "A synthetic prompt", "/synthetic/one");
    append(
        &path,
        r#"{"type":"custom-title","customTitle":"A synthetic title"}"#,
    );
    let listing = histories.list();
    assert_eq!(title(&listing, id), "A synthetic prompt");
    assert_eq!(listing.malformed_lines, 0);

    let listing = histories.list();
    assert_eq!(listing.files_read, 0);
    assert_eq!(title(&listing, id), "A synthetic prompt");

    append(&path, "\n");
    assert_eq!(title(&histories.list(), id), "A synthetic title");
}

#[test]
fn deleting_a_working_directory_orphans_its_session_without_reading_history() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000109";
    let dir = histories.dir.path().join("work");
    fs::create_dir(&dir).unwrap();
    histories.claude_session(id, "A synthetic prompt", &dir.display().to_string());
    // The directory is in a temp folder, so `is:temp` keeps its session.
    let in_temp = || {
        let roots = histories.roots();
        let query = Query::parse("is:temp", &roots.providers());
        search(&roots, &histories.data(), &query).unwrap()
    };
    let listing = in_temp();
    let [listed] = listing.sessions.as_slice() else {
        panic!("expected one session");
    };
    assert_eq!(listed.location.availability, Availability::Available);

    fs::remove_dir(&dir).unwrap();
    let listing = in_temp();
    assert_eq!(listing.files_read, 0);
    let [listed] = listing.sessions.as_slice() else {
        panic!("expected one session");
    };
    assert_eq!(listed.location.availability, Availability::Orphaned);
}

#[test]
fn rebuilding_gives_the_same_sessions_as_indexing_change_by_change() {
    let histories = Histories::new();
    let claude = "aaaaaaaa-0000-4000-8000-000000000110";
    let transcript = histories.claude_session(claude, "A synthetic prompt", "/synthetic/one");
    let subagents = transcript.with_extension("").join("subagents");
    fs::create_dir_all(&subagents).unwrap();
    for agent in ["agent-x", "agent-y"] {
        fs::write(
            subagents.join(format!("{agent}.jsonl")),
            claude_prompt("A synthetic subagent task", "/synthetic/one"),
        )
        .unwrap();
    }
    let codex = "eeeeeeee-0000-7000-8000-000000000110";
    histories.codex_session(codex, "A synthetic Codex prompt", "/synthetic/two");
    let session_index = histories.dir.path().join("codex/session_index.jsonl");
    fs::write(
        &session_index,
        format!(r#"{{"id":"{codex}","thread_name":"A first name"}}"#) + "\n",
    )
    .unwrap();
    histories.list();

    append(
        &transcript,
        concat!(
            r#"{"type":"ai-title","aiTitle":"A generated title"}"#,
            "\n",
            r#"{"type":"pr-link","prNumber":7}"#,
            "\n"
        ),
    );
    append(
        &session_index,
        &(format!(r#"{{"id":"{codex}","thread_name":"A second name"}}"#) + "\n"),
    );
    histories.codex_session(
        "eeeeeeee-0000-7000-8000-000000000111",
        "Another synthetic Codex prompt",
        "/synthetic/three",
    );
    fs::remove_file(subagents.join("agent-y.jsonl")).unwrap();
    let incremental = histories.list();

    let rebuilt = rebuild(&histories.roots(), &histories.data()).unwrap();
    let everything = |listing: &Listing| -> Vec<(Session, Location)> {
        listing
            .sessions
            .iter()
            .map(|listed| (listed.session.clone(), listed.location.clone()))
            .collect()
    };
    assert_eq!(everything(&rebuilt), everything(&incremental));
    assert_eq!(rebuilt.sessions.len(), 3);
    // The transcript, one subagent, two rollouts and the session index.
    assert_eq!(rebuilt.files_read, 5);
}

#[test]
fn rebuilding_leaves_the_preferences_untouched() {
    let histories = Histories::new();
    histories.claude_session(
        "aaaaaaaa-0000-4000-8000-000000000112",
        "A synthetic prompt",
        "/synthetic/one",
    );
    hide(
        &histories.roots(),
        &histories.data(),
        "aaaaaaaa-0000-4000-8000-000000000112",
    )
    .unwrap();
    let prefs = histories.data().prefs();
    let contents = fs::read(&prefs).unwrap();

    rebuild(&histories.roots(), &histories.data()).unwrap();
    assert_eq!(fs::read(&prefs).unwrap(), contents);
}

#[test]
fn preferences_filled_with_garbage_fail_the_listing_and_are_kept() {
    let histories = Histories::new();
    histories.claude_session(
        "aaaaaaaa-0000-4000-8000-000000000114",
        "A synthetic prompt",
        "/synthetic/one",
    );
    let prefs = histories.data().prefs();
    fs::create_dir_all(prefs.parent().unwrap()).unwrap();
    let contents = "synthetic garbage, not a database ".repeat(200);
    fs::write(&prefs, &contents).unwrap();

    let Err(error) = list(&histories.roots(), &histories.data()) else {
        panic!("a listing over unreadable preferences should fail");
    };
    assert_eq!(error.path, prefs);
    assert!(
        error.to_string().starts_with(&format!(
            "can't use the preferences at {}: ",
            prefs.display()
        )),
        "{error}"
    );
    assert_eq!(fs::read_to_string(&prefs).unwrap(), contents);
}

#[test]
fn an_index_filled_with_garbage_is_built_again() {
    let histories = Histories::new();
    let id = "aaaaaaaa-0000-4000-8000-000000000113";
    histories.claude_session(id, "A synthetic prompt", "/synthetic/one");
    let index = histories.data().index();
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, "synthetic garbage, not a database ".repeat(200)).unwrap();

    let listing = histories.list();
    assert_eq!(title(&listing, id), "A synthetic prompt");
    assert_eq!(listing.sessions.len(), 1);
}

#[test]
fn malformed_lines_are_still_counted_when_nothing_is_read() {
    let histories = Histories::new();
    let path = histories.claude_session(
        "aaaaaaaa-0000-4000-8000-000000000114",
        "A synthetic prompt",
        "/synthetic/one",
    );
    append(&path, "not json\nnot json either\n");
    assert_eq!(histories.list().malformed_lines, 2);

    let listing = histories.list();
    assert_eq!(listing.files_read, 0);
    assert_eq!(listing.malformed_lines, 2);
}

#[test]
fn a_codex_rename_changes_the_title_after_reading_only_the_session_index() {
    let histories = Histories::new();
    let renamed = "eeeeeeee-0000-7000-8000-000000000115";
    histories.codex_session(renamed, "A synthetic prompt", "/synthetic/one");
    histories.codex_session(
        "eeeeeeee-0000-7000-8000-000000000116",
        "Another synthetic prompt",
        "/synthetic/two",
    );
    let session_index = histories.dir.path().join("codex/session_index.jsonl");
    fs::write(&session_index, "").unwrap();
    assert_eq!(title(&histories.list(), renamed), "A synthetic prompt");

    append(
        &session_index,
        &(format!(r#"{{"id":"{renamed}","thread_name":"A synthetic name"}}"#) + "\n"),
    );
    let listing = histories.list();
    assert_eq!(title(&listing, renamed), "A synthetic name");
    assert_eq!(listing.files_read, 1);
}
