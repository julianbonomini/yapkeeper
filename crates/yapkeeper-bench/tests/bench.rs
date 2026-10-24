//! Runs the `yapkeeper-bench` command on a tiny synthetic corpus and checks
//! the shape of the report it prints, never its timings.

#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn a_tiny_run_reports_every_measurement_against_its_target() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yapkeeper-bench"))
        .args(["--sessions", "12", "--subagents", "2", "--megabytes", "1"])
        .args(["--runs", "3", "--seed", "5", "--dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    let line = |start: &str| {
        lines
            .iter()
            .find(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("no line starts with {start:?}:\n{stdout}"))
            .to_string()
    };
    let corpus = line("Corpus");
    assert!(corpus.contains("seed 5"), "{corpus}");
    assert!(corpus.contains("12 sessions"), "{corpus}");
    assert!(corpus.contains("14 files"), "{corpus}");
    // Every generated session is indexed, so the histories parse.
    assert!(line("Index").contains("12 sessions"), "{stdout}");
    assert!(line("Runs").contains("3 per measurement"), "{stdout}");
    let header = line("Measurement");
    for column in ["p50", "p95", "Target"] {
        assert!(header.contains(column), "{header}");
    }
    let launch = line("Launch, warm index");
    assert!(launch.contains("200 ms"), "{launch}");
    assert_eq!(launch.matches(" ms").count(), 3, "{launch}");
    let searches: Vec<&&str> = lines
        .iter()
        .filter(|line| line.starts_with("Search "))
        .collect();
    assert_eq!(searches.len(), 4, "{stdout}");
    for search in searches {
        assert!(search.contains("50 ms"), "{search}");
        assert_eq!(search.matches(" ms").count(), 3, "{search}");
    }
    // The corpus is removed when the run ends.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
