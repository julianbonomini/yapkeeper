//! Drives `yapkeeper::launch`, which replaces the calling process, so every
//! case re-runs its own test in a child of this test binary: the child
//! launches and the parent reads what it printed. Nothing here launches in
//! the test process itself, whose working directory every test shares.

#![forbid(unsafe_code)]
#![cfg(unix)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yapkeeper::{ResumeCommand, launch};

/// Set only in a child: the directory its test launches in.
const CHILD_CWD: &str = "YAPKEEPER_LAUNCH_CWD";

/// In the child `in_child` starts, launches `program` with `args` in the
/// directory the parent named, and when `launch` returns, prints
/// `error: <kind>` and exits. In the parent, does nothing.
fn launch_if_child(program: &str, args: &[&str]) {
    let Some(cwd) = env::var_os(CHILD_CWD) else {
        return;
    };
    let error = launch(&ResumeCommand {
        program: program.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        cwd: PathBuf::from(cwd),
    });
    println!("error: {:?}", error.kind());
    std::process::exit(0);
}

/// Re-runs only `test` in a child of this test binary, asking it to launch
/// in `cwd`, and returns each line the child printed.
fn in_child(test: &str, cwd: &Path) -> Vec<String> {
    // `--quiet` keeps the harness from printing the test's name before it
    // runs, on the line the launched program writes to.
    let output = Command::new(env::current_exe().unwrap())
        .args([
            test,
            "--exact",
            "--nocapture",
            "--test-threads=1",
            "--quiet",
        ])
        .env(CHILD_CWD, cwd)
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn the_program_starts_in_the_commands_directory() {
    launch_if_child("pwd", &["-P"]);
    let dir = tempfile::tempdir().unwrap();
    let printed = in_child("the_program_starts_in_the_commands_directory", dir.path());
    let canonical = fs::canonicalize(dir.path()).unwrap();
    assert!(
        printed.contains(&canonical.display().to_string()),
        "{printed:#?}"
    );
}

#[test]
fn arguments_reach_the_program_whole_with_no_shell() {
    launch_if_child("printf", &["%s\n", "$HOME", "a b; echo hi"]);
    let printed = in_child(
        "arguments_reach_the_program_whole_with_no_shell",
        Path::new("/"),
    );
    assert!(printed.contains(&"$HOME".to_string()), "{printed:#?}");
    assert!(
        printed.contains(&"a b; echo hi".to_string()),
        "{printed:#?}"
    );
    assert!(!printed.contains(&"hi".to_string()), "{printed:#?}");
}

#[test]
fn a_program_that_does_not_exist_is_not_found() {
    launch_if_child("yapkeeper-synthetic-missing-program", &[]);
    let printed = in_child("a_program_that_does_not_exist_is_not_found", Path::new("/"));
    assert!(
        printed.contains(&"error: NotFound".to_string()),
        "{printed:#?}"
    );
}

#[test]
fn a_directory_that_does_not_exist_says_why() {
    launch_if_child("pwd", &[]);
    let printed = in_child(
        "a_directory_that_does_not_exist_says_why",
        Path::new("/synthetic/gone"),
    );
    assert!(
        printed.iter().any(|line| line.starts_with("error: ")),
        "{printed:#?}"
    );
}
