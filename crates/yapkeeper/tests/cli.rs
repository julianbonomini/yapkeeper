//! Runs the `yapkeeper` binary with a cleared environment, so the only
//! history it can see is the synthetic one each test points it at.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(unix)]
use support::Locked;
use support::{repo_with_worktree, temp_dir};
use yapkeeper::{Catalog, DataDir, Roots, Settings, State};

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
    }
}

#[test]
fn no_arguments_without_a_terminal_print_help_and_say_the_palette_needs_one() {
    let home = tempfile::tempdir().unwrap();
    // `output()` gives the child a null stdin and a piped stdout.
    let output = yapkeeper(&[], home.path(), None);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let stderr = stderr(&output);
    assert!(stderr.contains("list"), "{stderr}");
    assert_eq!(
        stderr.lines().last(),
        Some(
            "yapkeeper: the palette needs a terminal; \
             yapkeeper list and yapkeeper search work in scripts"
        ),
        "{stderr}"
    );
}

#[test]
fn help_names_the_history_directories_the_environment_resolves() {
    let home = tempfile::tempdir().unwrap();
    let claude = home.path().join("c");
    let codex = home.path().join("x");
    for args in [&["--help"][..], &["list", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_yapkeeper"))
            .args(args)
            .env_clear()
            .env("HOME", home.path())
            .env("CLAUDE_CONFIG_DIR", &claude)
            .env("CODEX_HOME", &codex)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        let stdout = stdout(&output);
        for dir in [claude.join("projects"), codex.clone()] {
            let dir = dir.display().to_string();
            assert!(
                stdout.contains(&dir),
                "{args:?} should name {dir}: {stdout}"
            );
        }
    }
}

#[test]
fn help_names_the_default_history_directories_under_home() {
    let home = tempfile::tempdir().unwrap();
    for args in [&["--help"][..], &["list", "--help"]] {
        let output = yapkeeper(args, home.path(), None);
        assert!(output.status.success(), "{args:?}");
        let stdout = stdout(&output);
        for dir in [".claude/projects", ".codex"] {
            let dir = home.path().join(dir).display().to_string();
            assert!(
                stdout.contains(&dir),
                "{args:?} should name {dir}: {stdout}"
            );
        }
    }
}

#[test]
fn help_says_why_no_history_directory_resolves_without_a_home() {
    for args in [&["--help"][..], &["search", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_yapkeeper"))
            .args(args)
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        let stdout = stdout(&output);
        assert!(stdout.contains("nor HOME is set"), "{args:?}: {stdout}");
    }
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
    let home = temp_dir();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000020",
        "Work in my home",
        &home.path().join("code"),
        "",
    );

    let output = yapkeeper(&["search", "is:temp"], home.path(), None);
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
    let tmp = temp_dir();
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

    let output = yapkeeper(&["search", "is:temp"], home.path(), None);
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
    let tmp = temp_dir();
    let gone = tmp.path().join("gone");
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000042",
        "Work that was deleted",
        &gone,
        "",
    );

    let output = yapkeeper(&["search", "is:temp"], home.path(), None);
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
    let tmp = temp_dir();
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

    let output = yapkeeper(&["search", "is:temp"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let line = line_titled(&stdout, "Work behind a locked door");
    assert!(line.contains("  unavailable  "), "{line:?}");
    assert!(!line.contains("orphaned"), "{line:?}");
}

#[test]
fn list_leaves_out_the_label_column_when_every_session_is_available() {
    let home = tempfile::tempdir().unwrap();
    let tmp = temp_dir();
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

    let output = yapkeeper(&["search", "is:temp"], home.path(), None);
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
            "yapkeeper: can't use the index at {}: ",
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
fn search_help_names_every_state_and_provider_the_query_accepts() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper(&["search", "--help"], home.path(), None);
    assert!(output.status.success());
    let help = stdout(&output);
    let roots = Roots {
        claude: home.path().to_path_buf(),
        codex: home.path().to_path_buf(),
    };
    let states = State::ALL.map(|state| format!("is:{}", state.name()));
    let providers = roots.providers().into_iter().map(|p| format!("#{p}"));
    for phrase in states.into_iter().chain(providers) {
        assert!(help.contains(&phrase), "{phrase:?} is missing from {help}");
    }
    assert!(help.contains("'!temp'"), "{help}");
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
        "@where",
        "#provider",
        "is:state",
        "not:x",
        "quote ! for the shell",
        "left out unless the query has is:temp",
        "Orphaned sessions come after the rest",
    ] {
        assert!(help.contains(phrase), "{phrase:?} is missing from {help}");
    }

    let output = yapkeeper(&["--help"], home.path(), None);
    assert!(stdout(&output).contains("search"), "{}", stdout(&output));
}

#[test]
fn search_names_the_valid_choices_for_an_unknown_provider_or_state_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let output = yapkeeper_over_fixtures(&["search", "#pi", "is:bogus"], home.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    let stderr = stderr(&output);
    for hint in [
        "yapkeeper: \"#pi\" names no provider (claude, codex)\n",
        "yapkeeper: \"is:bogus\" names no state (orphaned, temp, git, folder, pr, recap, unavailable, hidden)\n",
    ] {
        assert!(stderr.contains(hint), "{hint:?} is missing from {stderr}");
    }
    // No word to find in tool-level text, so `--tools` can't help.
    assert!(!stderr.contains("--tools"), "{stderr}");
}

#[test]
fn list_leaves_out_temp_sessions_and_search_is_temp_prints_them() {
    let home = tempfile::tempdir().unwrap();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000206",
        "Try it out",
        Path::new("/tmp/yapkeeper-synthetic-scratch"),
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000207",
        "Keep this",
        Path::new("/synthetic/kept"),
        "",
    );

    let list = yapkeeper(&["list"], home.path(), None);
    assert!(list.status.success(), "{}", stderr(&list));
    let stdout_of_list = stdout(&list);
    assert_eq!(stdout_of_list.lines().count(), 1, "{stdout_of_list}");
    assert!(
        stdout_of_list.starts_with("Keep this  "),
        "{stdout_of_list}"
    );

    let temp = yapkeeper(&["search", "is:temp"], home.path(), None);
    assert!(temp.status.success(), "{}", stderr(&temp));
    let stdout = stdout(&temp);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(
        stdout.starts_with("Try it out  claude  ")
            && stdout.ends_with("  orphaned  /tmp/yapkeeper-synthetic-scratch\n"),
        "{stdout}"
    );
}

#[test]
fn search_by_provider_prints_only_that_providers_lines_in_the_format_of_list() {
    let home = tempfile::tempdir().unwrap();
    let cwd = Path::new("/synthetic/kept");
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000208",
        "Retry the claude upload",
        cwd,
        "",
    );
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000209",
        "Tidy the notes",
        cwd,
        "",
    );
    // Claude Code from `$HOME/.claude`, Codex from the committed fixtures.
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yapkeeper"))
            .args(args)
            .env_clear()
            .env("HOME", home.path())
            .env("CODEX_HOME", CODEX_FIXTURES)
            .output()
            .unwrap()
    };
    let words = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
    let list = run(&["list"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let list = stdout(&list);

    let output = run(&["search", "!codex", "retry"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout_of_search = stdout(&output);
    assert_eq!(stdout_of_search.lines().count(), 1, "{stdout_of_search}");
    assert!(
        stdout_of_search.starts_with("Retry the claude upload  claude  ")
            && stdout_of_search.ends_with("  orphaned  /synthetic/kept\n"),
        "{stdout_of_search}"
    );

    let output = run(&["search", "#claude"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let expected: Vec<String> = list
        .lines()
        .filter(|line| words(line).contains(" claude "))
        .map(words)
        .collect();
    assert_eq!(expected.len(), 2, "{list}");
    let found: Vec<String> = stdout(&output).lines().map(words).collect();
    assert_eq!(found, expected);
}

#[test]
fn list_with_only_temp_sessions_says_how_to_list_them() {
    let home = temp_dir();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000210",
        "Work in a temp folder",
        &home.path().join("scratch"),
        "",
    );

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: no sessions outside temp folders; yapkeeper search is:temp lists them\n"
    );
}

#[test]
fn search_with_only_an_unfinished_filter_over_temp_sessions_says_how_to_list_them() {
    let home = temp_dir();
    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000211",
        "Work in a temp folder",
        &home.path().join("scratch"),
        "",
    );

    let output = yapkeeper(&["search", "@"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: no sessions outside temp folders; yapkeeper search is:temp lists them\n"
    );
}

/// Every file under `dir` with its contents and modification time.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                files.push((path.clone(), fs::read(&path).unwrap(), modified));
            }
        }
    }
    files.sort();
    files
}

const LAUNCH_CHECKLIST: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn hide_leaves_a_session_out_until_unhide_and_never_touches_the_histories() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| yapkeeper_over_fixtures(args, home.path());
    let before = (
        snapshot(Path::new(FIXTURES)),
        snapshot(Path::new(CODEX_FIXTURES)),
    );
    let listed = stdout(&run(&["list"]));
    assert!(line_titled(&listed, "Launch checklist").contains("claude"));

    let output = run(&["hide", LAUNCH_CHECKLIST]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: hid \"Launch checklist\"; yapkeeper search is:hidden lists it\n"
    );
    let without = stdout(&run(&["list"]));
    assert!(!without.contains("Launch checklist"), "{without}");
    assert_eq!(without.lines().count(), listed.lines().count() - 1);
    assert!(!stdout(&run(&["search", "launch"])).contains("Launch checklist"));
    let hidden = stdout(&run(&["search", "is:hidden"]));
    assert_eq!(hidden.lines().count(), 1, "{hidden}");
    assert!(hidden.starts_with("Launch checklist"), "{hidden}");

    let output = run(&["unhide", LAUNCH_CHECKLIST]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "yapkeeper: unhid \"Launch checklist\"\n");
    assert_eq!(stdout(&run(&["list"])), listed);

    let after = (
        snapshot(Path::new(FIXTURES)),
        snapshot(Path::new(CODEX_FIXTURES)),
    );
    assert!(before == after, "hide or unhide changed a history file");
}

#[test]
fn hide_with_an_unknown_id_fails_and_says_where_ids_are_shown() {
    let home = tempfile::tempdir().unwrap();
    for command in ["hide", "unhide"] {
        let output = yapkeeper_over_fixtures(&[command, "no-such-session"], home.path());
        assert_eq!(output.status.code(), Some(1), "{command}");
        assert_eq!(stdout(&output), "");
        assert_eq!(
            stderr(&output),
            "yapkeeper: no session has the id \"no-such-session\"\n\
             yapkeeper list --ids shows each session's id\n"
        );
    }
}

#[test]
fn ids_puts_each_sessions_id_first_and_leaves_the_rest_of_the_line_as_it_was() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| yapkeeper_over_fixtures(args, home.path());
    let plain = stdout(&run(&["list"]));
    for args in [&["list", "--ids"][..], &["search", "--ids"]] {
        let with_ids = stdout(&run(args));
        assert_eq!(with_ids.lines().count(), plain.lines().count(), "{args:?}");
        for (line, plain) in with_ids.lines().zip(plain.lines()) {
            let (id, rest) = line.split_once("  ").unwrap();
            assert_eq!(id.len(), 36, "{line:?}");
            assert_eq!(rest, plain, "{args:?}");
        }
    }
    let launch = stdout(&run(&["search", "--ids", "launch"]));
    assert!(launch.starts_with(&format!("{LAUNCH_CHECKLIST}  Launch checklist")));
}

#[test]
fn list_with_every_session_hidden_or_temp_says_which_queries_list_them() {
    let home = temp_dir();
    let hidden = "aaaaaaaa-0000-4000-8000-000000000212";
    write_home_session(
        home.path(),
        hidden,
        "Work that is hidden",
        Path::new("/synthetic/gone"),
        "",
    );
    let run = |args: &[&str]| yapkeeper(args, home.path(), None);
    assert!(run(&["hide", hidden]).status.success());

    let output = run(&["list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: every session is hidden; yapkeeper search is:hidden lists them\n"
    );

    write_home_session(
        home.path(),
        "aaaaaaaa-0000-4000-8000-000000000213",
        "Work in a temp folder",
        &home.path().join("scratch"),
        "",
    );
    let output = run(&["list"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "yapkeeper: every session is hidden or in a temp folder; \
         yapkeeper search is:temp or is:hidden lists them\n"
    );
}

#[test]
fn list_keeps_the_documented_defaults_whatever_the_palette_is_set_to() {
    let home = tempfile::tempdir().unwrap();
    let scratch = temp_dir();
    write_home_session(home.path(), "a1", "Live work", Path::new("/usr"), "");
    write_home_session(home.path(), "a2", "Scratch work", scratch.path(), "");
    let orphaned = r#"{"type":"user","message":{"content":"Orphaned work"},"cwd":"/synthetic/gone","timestamp":"2026-10-21T10:00:00Z"}"#;
    let project = home.path().join(".claude/projects/-synthetic-temp");
    fs::write(project.join("a3.jsonl"), format!("{orphaned}\n")).unwrap();
    let roots = Roots {
        claude: home.path().join(".claude"),
        codex: home.path().join(".codex"),
    };
    let data = DataDir::at(home.path().join(".local/share/yapkeeper"));
    let mut catalog = Catalog::open(&roots, &data).unwrap();
    catalog
        .set_settings(Settings {
            hide_temp: false,
            orphans_last: false,
            ..Settings::default()
        })
        .unwrap();

    let output = yapkeeper(&["list"], home.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    let titles: Vec<&str> = stdout
        .lines()
        .map(|line| line.split("  ").next().unwrap())
        .collect();
    assert_eq!(titles, ["Live work", "Orphaned work"], "{stdout}");
}
