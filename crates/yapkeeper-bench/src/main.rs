//! The `yapkeeper-bench` command: writes a synthetic corpus in a fresh
//! folder, or reads the histories this environment names when asked to,
//! measures the palette over a temporary index, and prints one report on
//! stdout. The corpus and the index are deleted when it ends.

#![forbid(unsafe_code)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use yapkeeper::{DataDir, Location, RecordedGit, Roots};
use yapkeeper_bench::{Corpus, FRAME, Measurements, Shape, generate, measure};

/// The targets in docs/product.md: a usable launch and search results.
const LAUNCH_TARGET: Duration = Duration::from_millis(200);
const SEARCH_TARGET: Duration = Duration::from_millis(50);

const MEBIBYTE: u64 = 1024 * 1024;

/// Measure Yapkeeper's warm launch and search latency
///
/// By default, writes a synthetic corpus sized like real histories, builds an
/// index of it, and reports p50 and p95 against the 200 ms and 50 ms
/// targets for: opening the palette, from opening the cached catalog to its
/// first frame; typing each representative query a key at a time, naming
/// the prefix whose keystroke was slowest, from its key press to the frame
/// showing its results; and pasting a query, from the first key pressed to
/// the frame showing its results. Each typing and paste run starts from an
/// empty query.
#[derive(Parser)]
struct Bench {
    /// Seed the synthetic corpus is written from
    #[arg(long, default_value_t = Shape::default().seed)]
    seed: u64,
    /// Sessions in the synthetic corpus
    #[arg(long, default_value_t = Shape::default().sessions)]
    sessions: usize,
    /// Claude Code subagent transcripts in the synthetic corpus
    #[arg(long, default_value_t = Shape::default().subagents)]
    subagents: usize,
    /// Roughly how many MiB of history the synthetic corpus holds
    #[arg(long, default_value_t = Shape::default().bytes.div_ceil(MEBIBYTE))]
    megabytes: u64,
    /// Times each measurement is taken
    #[arg(long, default_value = "30")]
    runs: NonZeroUsize,
    /// Where the synthetic corpus is written, in a fresh folder deleted at
    /// the end; not under a system temp folder
    #[arg(long, default_value = ".")]
    dir: PathBuf,
    /// Measure the Claude Code, Codex and Pi histories this environment
    /// names instead, read-only, with a temporary index; prints only counts
    /// and timings
    #[arg(long, conflicts_with_all = ["seed", "sessions", "subagents", "megabytes", "dir"])]
    real_histories: bool,
}

fn main() -> ExitCode {
    let bench = Bench::parse();
    match run(&bench) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("yapkeeper-bench: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Measures what `bench` asks for and prints the report, or says why it can't.
fn run(bench: &Bench) -> Result<(), String> {
    // The index lives in its own private temporary folder, never the
    // person's own, so a run neither reads nor changes it.
    let index_dir =
        tempfile::tempdir().map_err(|error| format!("can't make a temporary index: {error}"))?;
    let data = DataDir::at(index_dir.path().to_path_buf());
    if bench.real_histories {
        let roots = Roots::from_env().map_err(|error| error.to_string())?;
        let measured = measure(&roots, &data, bench.runs).map_err(|error| error.to_string())?;
        println!(
            "Corpus       real histories, read-only: {} sessions",
            measured.indexed
        );
        report(&measured, bench.runs);
        return Ok(());
    }
    let corpus_dir = corpus_dir(&bench.dir)?;
    let shape = Shape {
        seed: bench.seed,
        sessions: bench.sessions,
        subagents: bench.subagents,
        bytes: bench.megabytes * MEBIBYTE,
    };
    let corpus = generate(&shape, corpus_dir.path()).map_err(|error| {
        format!(
            "can't write the corpus in {}: {error}",
            corpus_dir.path().display()
        )
    })?;
    let measured = measure(&corpus.roots, &data, bench.runs).map_err(|error| error.to_string())?;
    describe(&corpus, shape.seed);
    report(&measured, bench.runs);
    Ok(())
}

/// A fresh folder in `dir` for the synthetic corpus, deleted when dropped.
/// Fails when `dir` sits under a system temp folder, where every live
/// session would count as temp, or the folder can't be made.
fn corpus_dir(dir: &Path) -> Result<tempfile::TempDir, String> {
    let dir = std::path::absolute(dir)
        .map_err(|error| format!("can't use {}: {error}", dir.display()))?;
    if Location::of(&dir, &RecordedGit::default()).temp {
        return Err(format!(
            "{} is under a system temp folder, where every session counts as temp; pass --dir elsewhere",
            dir.display()
        ));
    }
    tempfile::Builder::new()
        .prefix(".yapkeeper-bench-")
        .tempdir_in(&dir)
        .map_err(|error| format!("can't make the corpus folder in {}: {error}", dir.display()))
}

/// The corpus's size and the parameters it was written from.
fn describe(corpus: &Corpus, seed: u64) {
    println!(
        "Corpus       synthetic, seed {seed}: {} sessions ({} Claude Code, {} Codex, {} Pi), {} files, {:.1} MiB",
        corpus.sessions(),
        corpus.claude,
        corpus.codex,
        corpus.pi,
        corpus.files,
        corpus.bytes as f64 / MEBIBYTE as f64
    );
    println!(
        "             {} orphaned, {} in temp folders, {} subagent transcripts",
        corpus.orphaned,
        corpus.temp,
        corpus.subagents()
    );
}

/// The index, the timing boundaries, then one row per measurement: its
/// percentiles beside its target, marked `over` when its p95 misses it.
fn report(measured: &Measurements, runs: NonZeroUsize) {
    println!(
        "Index        {} sessions, built from nothing in {}",
        measured.indexed,
        millis(measured.warmed)
    );
    println!(
        "Runs         {runs} per measurement, each up to the frame showing its results at {}x{}",
        FRAME.0, FRAME.1
    );
    let mut rows = vec![(
        "Launch, warm index".to_string(),
        measured.launch,
        LAUNCH_TARGET,
    )];
    for slowest in &measured.slowest_keystrokes {
        rows.push((
            format!("Search \"{}\" at \"{}\"", slowest.query, slowest.prefix),
            slowest.latency,
            SEARCH_TARGET,
        ));
    }
    rows.push((
        format!("Paste \"{}\"", measured.paste.query),
        measured.paste.latency,
        SEARCH_TARGET,
    ));
    // The name column is as wide as the longest name, so the rows fit in 80
    // columns unless a slowest prefix is unusually long.
    let name = rows
        .iter()
        .map(|(name, ..)| name.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    println!();
    println!(
        "{:<name$}{:>10}{:>10}{:>10}",
        "Measurement", "p50", "p95", "Target"
    );
    for (measurement, latency, target) in rows {
        let verdict = match latency.p95 <= target {
            true => "",
            false => "  over",
        };
        println!(
            "{measurement:<name$}{:>10}{:>10}{:>10}{verdict}",
            millis(latency.p50),
            millis(latency.p95),
            format!("{} ms", target.as_millis())
        );
    }
}

/// `duration` in milliseconds, to a tenth below 100 ms.
fn millis(duration: Duration) -> String {
    let ms = duration.as_secs_f64() * 1000.0;
    match ms < 100.0 {
        true => format!("{ms:.1} ms"),
        false => format!("{ms:.0} ms"),
    }
}
