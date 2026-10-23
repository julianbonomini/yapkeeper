//! Runs the `yapkeeper` binary with a cleared environment, so the only
//! history it can see is the synthetic one each test points it at.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-claude/tests/fixtures/claude"
);

fn yapkeeper(args: &[&str], home: &Path, claude_config_dir: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yapkeeper"));
    command.args(args).env_clear().env("HOME", home);
    if let Some(dir) = claude_config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    }
    command.output().unwrap()
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
