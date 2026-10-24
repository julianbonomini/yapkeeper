//! Drives `yapkeeper::list` over synthetic sessions that point into real Git
//! repos and linked worktrees built in the system temp folder.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

#[cfg(unix)]
use support::Locked;
use support::{main_checkout, repo_with_worktree, temp_dir};
use tempfile::TempDir;
use yapkeeper::{Availability, DataDir, Location, Place, Query, RecordedGit, Roots, search};

mod support;

/// A fresh `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, each empty.
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

    /// One Claude Code session in `cwd`, with `gitBranch` when one is given.
    fn claude_session(&self, cwd: &Path, git_branch: Option<&str>) {
        let project = self.dir.path().join("claude/projects/-synthetic-location");
        fs::create_dir_all(&project).unwrap();
        let branch = git_branch
            .map(|branch| format!(r#","gitBranch":"{branch}""#))
            .unwrap_or_default();
        let record = format!(
            r#"{{"type":"user","message":{{"content":"A synthetic prompt"}},"cwd":"{}"{branch},"timestamp":"2026-10-20T10:00:00Z"}}"#,
            cwd.display()
        );
        fs::write(
            project.join("aaaaaaaa-0000-4000-8000-000000000030.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
    }

    /// One Codex session in `cwd` whose `session_meta` records `git`, a JSON object.
    fn codex_session(&self, cwd: &Path, git: &str) {
        let day = self.dir.path().join("codex/sessions/2026/10/22");
        fs::create_dir_all(&day).unwrap();
        let meta = format!(
            r#"{{"timestamp":"2026-10-22T08:00:00Z","type":"session_meta","payload":{{"id":"eeeeeeee-0000-7000-8000-000000000030","cwd":"{}","source":"cli","git":{git}}}}}"#,
            cwd.display()
        );
        let prompt = r#"{"timestamp":"2026-10-22T08:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"A synthetic prompt"}]}}"#;
        fs::write(
            day.join("rollout-2026-10-22T08-00-00-synthetic.jsonl"),
            format!("{meta}\n{prompt}\n"),
        )
        .unwrap();
    }

    /// The location of the only session listed. Every session here is in a
    /// temp folder, so `is:temp` finds them all.
    fn location(&self) -> Location {
        let roots = self.roots();
        let query = Query::parse("is:temp", &roots.providers());
        let data = DataDir::at(self.dir.path().join("data"));
        let listing = search(&roots, &data, &query).unwrap();
        let [listed] = listing.sessions.as_slice() else {
            panic!("expected one session, got {}", listing.sessions.len());
        };
        listed.location.clone()
    }
}

#[test]
fn a_session_in_a_main_checkout_is_named_after_its_folder() {
    let tmp = temp_dir();
    let repo = main_checkout(tmp.path());
    let histories = Histories::new();
    histories.claude_session(&repo.join("src"), None);

    let location = histories.location();
    assert_eq!(
        location.place,
        Place::MainCheckout {
            project: "acme-api".to_string()
        }
    );
    assert_eq!(location.availability, Availability::Available);
}

#[test]
fn a_session_in_a_linked_worktree_is_grouped_under_its_main_checkouts_project() {
    let tmp = temp_dir();
    let (_, worktree) = repo_with_worktree(tmp.path());
    let histories = Histories::new();
    histories.claude_session(&worktree, None);

    let location = histories.location();
    assert_eq!(
        location.place,
        Place::Worktree {
            project: "acme-api".to_string(),
            worktree: "billing-retry".to_string()
        }
    );
    assert_eq!(location.availability, Availability::Available);
}

#[test]
fn a_session_outside_any_repo_is_a_folder() {
    let tmp = temp_dir();
    let folder = tmp.path().join("notes");
    fs::create_dir(&folder).unwrap();
    let histories = Histories::new();
    histories.claude_session(&folder, None);

    let location = histories.location();
    assert_eq!(location.place, Place::Folder);
    assert_eq!(location.availability, Availability::Available);
}

#[test]
fn deleting_a_directory_after_it_was_listed_makes_its_session_orphaned() {
    let tmp = temp_dir();
    let (_, worktree) = repo_with_worktree(tmp.path());
    let histories = Histories::new();
    histories.claude_session(&worktree, None);
    assert_eq!(histories.location().availability, Availability::Available);

    fs::remove_dir_all(&worktree).unwrap();
    assert_eq!(histories.location().availability, Availability::Orphaned);
}

#[cfg(unix)]
#[test]
fn a_session_under_a_directory_it_may_not_enter_is_unavailable_not_orphaned() {
    let tmp = temp_dir();
    let locked = tmp.path().join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    let histories = Histories::new();
    histories.claude_session(&locked.join("inner"), None);

    let locked = Locked::new(&locked);
    if !locked.denies_access() {
        return;
    }
    assert_eq!(histories.location().availability, Availability::Unavailable);
}

#[cfg(unix)]
#[test]
fn a_session_in_a_directory_it_may_not_read_is_unavailable_not_orphaned() {
    let tmp = temp_dir();
    let locked = tmp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    let histories = Histories::new();
    histories.claude_session(&locked, None);

    let locked = Locked::new(&locked);
    if !locked.denies_access() {
        return;
    }
    assert_eq!(histories.location().availability, Availability::Unavailable);
}

#[test]
fn only_directories_under_a_system_temp_folder_are_temp() {
    let recorded = RecordedGit::default();
    for dir in [
        "/tmp/x",
        "/private/tmp/x",
        "/var/folders/x",
        "/private/var/folders/x",
    ] {
        assert!(Location::of(Path::new(dir), &recorded).temp, "{dir}");
    }
    for dir in ["/tmpfoo/x", "/synthetic/beta"] {
        assert!(!Location::of(Path::new(dir), &recorded).temp, "{dir}");
    }
}

#[test]
fn an_orphaned_codex_session_takes_its_project_from_the_recorded_remote() {
    let tmp = temp_dir();
    let histories = Histories::new();
    histories.codex_session(
        &tmp.path().join("gone/billing-retry"),
        r#"{"branch":"feature/billing-retry","repository_url":"git@example.com:acme/acme-api.git"}"#,
    );

    let location = histories.location();
    assert_eq!(location.availability, Availability::Orphaned);
    assert_eq!(
        location.place,
        Place::Worktree {
            project: "acme-api".to_string(),
            worktree: "billing-retry".to_string()
        }
    );
}

#[test]
fn an_orphaned_claude_session_takes_its_worktree_from_a_branch_matching_its_folder() {
    let tmp = temp_dir();
    let repo = main_checkout(tmp.path());
    let gone = repo.join("billing-retry");
    let histories = Histories::new();

    histories.claude_session(&gone, Some("feature/billing-retry"));
    let location = histories.location();
    assert_eq!(location.availability, Availability::Orphaned);
    assert_eq!(
        location.place,
        Place::Worktree {
            project: "acme-api".to_string(),
            worktree: "billing-retry".to_string()
        }
    );

    histories.claude_session(&gone, None);
    assert_eq!(
        histories.location().place,
        Place::MainCheckout {
            project: "acme-api".to_string()
        }
    );
}

#[test]
fn an_orphaned_session_with_nothing_to_infer_from_falls_back_to_its_folder() {
    let tmp = temp_dir();
    let histories = Histories::new();
    histories.claude_session(&tmp.path().join("gone/plain"), None);

    let location = histories.location();
    assert_eq!(location.availability, Availability::Orphaned);
    assert_eq!(location.place, Place::Folder);
}

#[test]
fn an_orphaned_session_in_a_worktree_folder_layout_takes_its_names_from_the_path() {
    for layout in [
        "acme-api.wt/billing-retry",
        ".tool/worktrees/acme-api/billing-retry",
        "acme-api/.claude/worktrees/billing-retry/src",
    ] {
        let tmp = temp_dir();
        let histories = Histories::new();
        histories.claude_session(&tmp.path().join(layout), None);

        let location = histories.location();
        assert_eq!(location.availability, Availability::Orphaned, "{layout}");
        assert_eq!(
            location.place,
            Place::Worktree {
                project: "acme-api".to_string(),
                worktree: "billing-retry".to_string()
            },
            "{layout}"
        );
    }
}

#[cfg(unix)]
#[test]
fn is_unavailable_finds_a_session_whose_directory_it_may_not_enter() {
    let histories = Histories::new();
    let tmp = temp_dir();
    let locked = tmp.path().join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    let open = tmp.path().join("open");
    fs::create_dir(&open).unwrap();
    histories.claude_session(&locked.join("inner"), None);
    histories.codex_session(&open, "{}");
    let locked = Locked::new(&locked);
    if !locked.denies_access() {
        return;
    }

    let roots = histories.roots();
    let query = Query::parse("is:temp is:unavailable", &roots.providers());
    let listing = search(
        &roots,
        &DataDir::at(histories.dir.path().join("data")),
        &query,
    )
    .unwrap();
    let providers: Vec<&str> = listing
        .sessions
        .iter()
        .map(|listed| listed.session.provider)
        .collect();
    assert_eq!(providers, ["claude"]);
}
