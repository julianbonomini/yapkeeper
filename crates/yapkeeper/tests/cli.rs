//! Runs the `yapkeeper` binary with a cleared environment, so the only
//! history it can see is the synthetic one each test points it at.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[cfg(unix)]
use support::Locked;
use support::repo_with_worktree;

mod support;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-claude/tests/fixtures/claude"
);
const CODEX_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-codex/tests/fixtures/codex"
);

fn yapkeeper(args: &[&str], home: &Path, claude_config_dir: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yapkeeper"));
    command.args(args).env_clear().env("HOME", home);
    if let Some(dir) = claude_config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    }
    command.output().unwrap()
}

/// Runs `yapkeeper` over both committed fixture histories.
fn yapkeeper_over_fixtures(args: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yapkeeper"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("CLAUDE_CONFIG_DIR", FIXTURES)
        .env("CODEX_HOME", CODEX_FIXTURES)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// An age such as `5m`, `3h`, `12d` or `40w`.
fn is_age(token: &str) -> bool {
    let Some(digits) = token.strip_suffix(['m', 'h', 'd', 'w']) else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

#[test]
fn list_prints_one_line_per_session_most_recent_first() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper(&["list"], home.path(), Some(Path::new(FIXTURES)));
    assert!(output.status.success(), "{}", stderr(&output));

    let stdout = stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    let expected = [
        ("Fix synthetic build failure", "/synthetic/beta"),
        ("Launch checklist", "/synthetic/alpha"),
        ("Summarise the synthetic release notes", "/synthetic/beta"),
        ("Tidy the synthetic notes folder", "/synthetic/gamma"),
    ];
    assert_eq!(lines.len(), expected.len(), "{stdout}");
    for (line, (title, dir)) in lines.iter().zip(expected) {
        assert!(
            line.starts_with(title),
            "{line:?} should start with {title:?}"
        );
        assert!(line.ends_with(dir), "{line:?} should end with {dir:?}");
        let tokens: Vec<&str> = line.split_whitespace().collect();
        assert!(tokens.contains(&"claude"), "{line:?} names no provider");
        assert!(
            tokens.iter().any(|token| is_age(token)),
            "{line:?} has no age"
        );
    }
}

#[test]
fn list_reads_home_dot_claude_when_claude_config_dir_is_unset() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-synthetic-beta");
    fs::create_dir_all(&project).unwrap();
    let file = "22222222-2222-4222-8222-222222222222.jsonl";
    fs::copy(
        Path::new(FIXTURES)
            .join("projects/-synthetic-beta")
            .join(file),
        project.join(file),
    )
    .unwrap();

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(
        stdout.starts_with("Fix synthetic build failure"),
        "{stdout}"
    );
}

#[test]
fn list_with_no_history_says_where_it_looked_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("no-claude-here");
    let output = yapkeeper(&["list"], home.path(), Some(&missing));

    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
    let stderr = stderr(&output);
    assert!(stderr.contains("no sessions found in"), "{stderr}");
    assert!(
        stderr.contains(&missing.join("projects").display().to_string()),
        "{stderr}"
    );
    assert!(
        stderr.contains(&home.path().join(".codex").display().to_string()),
        "{stderr}"
    );
}

#[test]
fn list_shows_claude_code_and_codex_sessions_together_in_aligned_columns() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper_over_fixtures(&["list"], home.path());
    assert!(output.status.success(), "{}", stderr(&output));

    let stdout = stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    let expected = [
        ("Fork of the retry policy", "codex", "/synthetic/epsilon"),
        ("Fix synthetic build failure", "claude", "/synthetic/beta"),
        (
            "Tune the synthetic retry policy",
            "codex",
            "/synthetic/delta",
        ),
        ("Launch checklist", "claude", "/synthetic/alpha"),
        (
            "Summarise the synthetic release notes",
            "claude",
            "/synthetic/beta",
        ),
        (
            "Tidy the synthetic notes folder",
            "claude",
            "/synthetic/gamma",
        ),
        (
            "Archive the synthetic changelog",
            "codex",
            "/synthetic/zeta",
        ),
    ];
    assert_eq!(lines.len(), expected.len(), "{stdout}");
    let provider_column = lines[0].find("  codex ").unwrap();
    for (line, (title, provider, dir)) in lines.iter().zip(expected) {
        assert!(
            line.starts_with(title),
            "{line:?} should start with {title:?}"
        );
        assert_eq!(
            line.find(&format!("  {provider} ")),
            Some(provider_column),
            "{line:?} should name {provider:?} in the provider column"
        );
        assert!(line.ends_with(dir), "{line:?} should end with {dir:?}");
    }
}

#[test]
fn help_describes_the_list_command() {
    let home = tempfile::tempdir().unwrap();
    for args in [&["--help"][..], &["list", "--help"]] {
        let output = yapkeeper(args, home.path(), None);
        assert!(output.status.success(), "{args:?}");
        let stdout = stdout(&output);
        assert!(stdout.contains("list"), "{args:?}: {stdout}");
        assert!(stdout.contains("CLAUDE_CONFIG_DIR"), "{args:?}: {stdout}");
        assert!(stdout.contains("CODEX_HOME"), "{args:?}: {stdout}");
    }

    let output = yapkeeper(&[], home.path(), None);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("list"), "{}", stderr(&output));
}

#[test]
fn list_prints_a_title_with_newlines_and_escapes_on_one_safe_line() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-synthetic-temp");
    fs::create_dir_all(&project).unwrap();
    // JSON escapes put a real newline and real ESC bytes into the prompt.
    let record = r#"{"type":"user","message":{"content":"Line one\nline two \u001b[31mred\u001b[0m"},"cwd":"/synthetic/temp","timestamp":"2026-10-20T10:00:00Z"}"#;
    fs::write(
        project.join("aaaaaaaa-0000-4000-8000-000000000017.jsonl"),
        format!("{record}\n"),
    )
    .unwrap();

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!output.stdout.contains(&0x1b));
    let stdout = stdout(&output);
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    assert!(
        stdout.starts_with("Line one line two [31mred[0m"),
        "{stdout:?}"
    );
}

/// Writes one session whose first prompt is `prompt` under `$HOME/.claude`.
fn write_home_session(home: &Path, id: &str, prompt: &str, cwd: &Path, extra_lines: &str) {
    let project = home.join(".claude/projects/-synthetic-temp");
    fs::create_dir_all(&project).unwrap();
    let record = format!(
        r#"{{"type":"user","message":{{"content":"{prompt}"}},"cwd":"{}","timestamp":"2026-10-20T10:00:00Z"}}"#,
        cwd.display()
    );
    fs::write(
        project.join(format!("{id}.jsonl")),
        format!("{record}\n{extra_lines}"),
    )
    .unwrap();
}

#[test]
fn list_cuts_long_titles_to_48_cells_and_lines_up_the_columns() {
    let home = tempfile::tempdir().unwrap();
    let cwd = Path::new("/synthetic/temp");
    // Each '長' takes two cells, so 30 of them are 60 cells wide.
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000018",
        &"長".repeat(30),
        cwd,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000019",
        "Short",
        cwd,
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    // 23 wide chars and the ellipsis fill 47 cells; a 24th wouldn't fit in 48.
    let long = format!("{}…", "長".repeat(23));
    let short = format!("Short{}", " ".repeat(42));
    assert_eq!(lines.len(), 2, "{stdout}");
    for (line, title) in lines.iter().zip([&long, &short]) {
        assert!(
            line.starts_with(&format!("{title}  claude  ")),
            "{line:?} should start with {title:?} and the provider column"
        );
    }
}

#[test]
fn list_shows_the_home_directory_as_a_tilde() {
    let home = tempfile::tempdir().unwrap();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000020",
        "Work in my home",
        &home.path().join("code"),
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    assert!(stdout.ends_with("  ~/code\n"), "{stdout:?}");
}

#[test]
fn list_reports_how_many_malformed_lines_it_skipped() {
    let home = tempfile::tempdir().unwrap();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000021",
        "A prompt before a bad line",
        Path::new("/synthetic/temp"),
        "this line is not json\n",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).lines().count(), 1);
    assert_eq!(stderr(&output), "yapkeeper: skipped 1 malformed line\n");
}

/// The one line of `stdout` that starts with `title`.
fn line_titled<'a>(stdout: &'a str, title: &str) -> &'a str {
    stdout
        .lines()
        .find(|line| line.starts_with(title))
        .unwrap_or_else(|| panic!("no line titled {title:?} in {stdout}"))
}

#[test]
fn list_shows_a_worktree_as_project_and_worktree_and_a_main_checkout_as_project() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (repo, worktree) = repo_with_worktree(tmp.path());
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000040",
        "Work in the worktree",
        &worktree,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000041",
        "Work in the main checkout",
        &repo,
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    for (title, location) in [
        ("Work in the worktree", "  acme-api › billing-retry"),
        ("Work in the main checkout", "  acme-api"),
    ] {
        let line = line_titled(&stdout, title);
        assert!(
            line.ends_with(location),
            "{line:?} should end with {location:?}"
        );
        assert!(
            !line.contains("orphaned") && !line.contains("unavailable"),
            "{line:?}"
        );
    }
}

#[test]
fn list_labels_a_session_whose_directory_is_gone_as_orphaned() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("gone");
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000042",
        "Work that was deleted",
        &gone,
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let line = line_titled(&stdout, "Work that was deleted");
    let location = gone.display().to_string();
    assert!(
        line.ends_with(&format!("  orphaned  {location}")),
        "{line:?} should label {location:?} orphaned"
    );
}

#[cfg(unix)]
#[test]
fn list_labels_a_session_whose_directory_it_may_not_enter_as_unavailable() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let locked = tmp.path().join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000043",
        "Work behind a locked door",
        &locked.join("inner"),
        "",
    );
    let locked = Locked::new(&locked);
    if !locked.denies_access() {
        return;
    }

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let line = line_titled(&stdout, "Work behind a locked door");
    assert!(line.contains("  unavailable  "), "{line:?}");
    assert!(!line.contains("orphaned"), "{line:?}");
}

#[test]
fn list_leaves_out_the_label_column_when_every_session_is_available() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (repo, _) = repo_with_worktree(tmp.path());
    let folder = tmp.path().join("notes");
    fs::create_dir(&folder).unwrap();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000044",
        "Work in the main checkout",
        &repo,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000045",
        "Work in a plain folder",
        &folder,
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let folder = folder.display().to_string();
    for (title, location) in [
        ("Work in the main checkout", "acme-api"),
        ("Work in a plain folder", folder.as_str()),
    ] {
        let line = line_titled(&stdout, title);
        let (before, after) = line.split_at(line.len() - location.len());
        assert_eq!(after, location, "{line:?} should end with {location:?}");
        let age = before
            .strip_suffix("  ")
            .and_then(|rest| rest.rsplit(' ').next());
        assert!(
            age.is_some_and(is_age),
            "{line:?} should follow the age with exactly two spaces"
        );
    }
}

/// Runs `yapkeeper` with only `HOME` and, when given, `XDG_DATA_HOME` set.
fn yapkeeper_with_data_home(args: &[&str], home: &Path, data_home: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yapkeeper"));
    command.args(args).env_clear().env("HOME", home);
    if let Some(dir) = data_home {
        command.env("XDG_DATA_HOME", dir);
    }
    command.output().unwrap()
}

#[test]
fn paths_prints_the_index_and_preferences_under_xdg_data_home_and_creates_neither() {
    let home = tempfile::tempdir().unwrap();
    let data_home = home.path().join("data");
    let output = yapkeeper_with_data_home(&["paths"], home.path(), Some(&data_home));
    assert!(output.status.success(), "{}", stderr(&output));

    let dir = data_home.join("yapkeeper");
    assert_eq!(
        stdout(&output),
        format!(
            "index        {}\npreferences  {}\n",
            dir.join("index.sqlite").display(),
            dir.join("prefs.sqlite").display()
        )
    );
    assert!(!dir.exists());
}

#[test]
fn paths_falls_back_to_home_when_xdg_data_home_is_unset_or_relative() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".local/share/yapkeeper");
    let expected = format!(
        "index        {}\npreferences  {}\n",
        dir.join("index.sqlite").display(),
        dir.join("prefs.sqlite").display()
    );
    for data_home in [None, Some(Path::new("relative/data"))] {
        let output = yapkeeper_with_data_home(&["paths"], home.path(), data_home);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stdout(&output), expected, "XDG_DATA_HOME={data_home:?}");
    }
}

#[test]
fn help_shows_where_the_index_and_preferences_are_kept() {
    let home = tempfile::tempdir().unwrap();
    let data_home = home.path().join("data");
    let dir = data_home.join("yapkeeper");
    for args in [&["--help"][..], &["list", "--help"]] {
        let output = yapkeeper_with_data_home(args, home.path(), Some(&data_home));
        assert!(output.status.success(), "{args:?}");
        let stdout = stdout(&output);
        for file in ["index.sqlite", "prefs.sqlite"] {
            let path = dir.join(file).display().to_string();
            assert!(
                stdout.contains(&path),
                "{args:?} should name {path}: {stdout}"
            );
        }
    }
}

#[test]
fn rebuild_reports_on_stderr_and_list_prints_the_same_afterwards() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| yapkeeper_over_fixtures(args, home.path());
    let before = run(&["list"]);
    assert!(before.status.success(), "{}", stderr(&before));

    let output = run(&["rebuild"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let index = home.path().join(".local/share/yapkeeper/index.sqlite");
    assert_eq!(
        stderr(&output),
        format!(
            "yapkeeper: rebuilt the index at {} (7 sessions)\n",
            index.display()
        )
    );

    let after = run(&["list"]);
    assert_eq!(stdout(&after), stdout(&before));
}

#[test]
fn list_fails_naming_the_index_when_it_cannot_be_written() {
    let home = tempfile::tempdir().unwrap();
    let not_a_dir = home.path().join("data");
    fs::write(&not_a_dir, "a regular file").unwrap();

    let output = yapkeeper_with_data_home(&["list"], home.path(), Some(&not_a_dir));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let stderr = stderr(&output);
    let index = not_a_dir.join("yapkeeper/index.sqlite");
    assert!(
        stderr.starts_with(&format!(
            "yapkeeper: can't write the index at {}: ",
            index.display()
        )),
        "{stderr}"
    );
    assert_eq!(
        stderr.lines().last(),
        Some("Check its permissions, or set XDG_DATA_HOME.")
    );
}

#[test]
fn search_prints_a_session_whose_prompt_contains_the_word_in_any_case() {
    let home = tempfile::tempdir().unwrap();
    let cwd = Path::new("/synthetic/temp");
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000201",
        "Retrying the upload",
        cwd,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000202",
        "Tidy the notes",
        cwd,
        "",
    );

    let output = yapkeeper(&["search", "retry"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(
        stdout.starts_with("Retrying the upload  claude  "),
        "{stdout}"
    );
}

#[test]
fn search_with_an_uppercase_letter_matches_case() {
    let home = tempfile::tempdir().unwrap();
    let cwd = Path::new("/synthetic/temp");
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000203",
        "Keep retrying the upload",
        cwd,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000204",
        "Retry the upload",
        cwd,
        "",
    );

    let output = yapkeeper(&["search", "Retry"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.starts_with("Retry the upload  claude  "), "{stdout}");
}

#[test]
fn search_finds_text_only_in_a_tool_call_with_tools_before_or_after_the_words() {
    let home = tempfile::tempdir().unwrap();
    let tool_call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/synthetic/zebra.log"}}]},"cwd":"/synthetic/temp"}"#;
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000205",
        "Read the log",
        Path::new("/synthetic/temp"),
        &format!("{tool_call}\n"),
    );

    let without = yapkeeper(&["search", "zebra"], home.path(), None);
    assert!(without.status.success(), "{}", stderr(&without));
    assert_eq!(stdout(&without), "");

    let before = yapkeeper(&["search", "--tools", "zebra"], home.path(), None);
    assert!(before.status.success(), "{}", stderr(&before));
    assert!(
        stdout(&before).starts_with("Read the log  claude  "),
        "{}",
        stdout(&before)
    );
    let after = yapkeeper(&["search", "zebra", "--tools"], home.path(), None);
    assert!(after.status.success(), "{}", stderr(&after));
    assert_eq!(stdout(&after), stdout(&before));
}

#[test]
fn search_with_no_words_prints_exactly_what_list_prints() {
    let home = tempfile::tempdir().unwrap();
    let list = yapkeeper_over_fixtures(&["list"], home.path());
    assert!(list.status.success(), "{}", stderr(&list));
    assert_eq!(stdout(&list).lines().count(), 7);
    for args in [&["search"][..], &["search", ""]] {
        let output = yapkeeper_over_fixtures(args, home.path());
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert_eq!(stdout(&output), stdout(&list), "{args:?}");
    }
}

#[test]
fn search_prints_its_matches_in_the_order_and_line_format_of_list() {
    let home = tempfile::tempdir().unwrap();
    let list = yapkeeper_over_fixtures(&["list"], home.path());
    let output = yapkeeper_over_fixtures(&["search", "retry"], home.path());
    assert!(output.status.success(), "{}", stderr(&output));

    let words = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
    let expected: Vec<String> = stdout(&list)
        .lines()
        .filter(|line| line.contains("retry"))
        .map(words)
        .collect();
    let stdout = stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.iter().map(|line| words(line)).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(expected.len(), 2, "{stdout}");
    let provider_column = lines[0].find("  codex ").unwrap();
    for line in &lines {
        assert_eq!(line.find("  codex "), Some(provider_column), "{line:?}");
    }
}

#[test]
fn search_matching_nothing_says_so_on_stderr_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper_over_fixtures(&["search", "no-such-word"], home.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        concat!(
            "yapkeeper: no sessions match \"no-such-word\"\n",
            "yapkeeper: --tools also searches tool calls, tool output and subagent transcripts\n",
            "yapkeeper: skipped 1 malformed line\n",
        )
    );

    let output = yapkeeper_over_fixtures(&["search", "--tools", "no-such-word"], home.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: no sessions match \"no-such-word\"\nyapkeeper: skipped 1 malformed line\n"
    );

    let empty = tempfile::tempdir().unwrap();
    let missing = empty.path().join("no-claude-here");
    let output = yapkeeper(&["search", "anything"], empty.path(), Some(&missing));
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let stderr = stderr(&output);
    assert!(stderr.contains("no sessions found in"), "{stderr}");
    assert!(!stderr.contains("match"), "{stderr}");
}

#[test]
fn search_help_describes_words_case_tools_and_the_double_dash() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper(&["search", "--help"], home.path(), None);
    assert!(output.status.success());
    let help = stdout(&output);
    for phrase in [
        "every word must match",
        "uppercase letter matches case",
        "--tools",
        "tool output",
        "subagent transcripts",
        "Put -- before a word that starts with -",
    ] {
        assert!(help.contains(phrase), "{phrase:?} is missing from {help}");
    }

    let output = yapkeeper(&["--help"], home.path(), None);
    assert!(stdout(&output).contains("search"), "{}", stdout(&output));
}
