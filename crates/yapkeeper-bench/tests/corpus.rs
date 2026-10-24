//! Checks that `yapkeeper_bench::generate` writes the same corpus for the
//! same seed, byte for byte, and a different one for another seed.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use yapkeeper_bench::{Shape, generate};

fn tiny(seed: u64) -> Shape {
    Shape {
        seed,
        sessions: 20,
        subagents: 3,
        bytes: 200_000,
    }
}

/// Every file under `dir`, by its path inside `dir`, with its contents.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_path_buf();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn the_same_seed_writes_the_same_corpus() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let corpus = dir.path().join("corpus");
    generate(&tiny(7), &corpus).unwrap();
    let first = files(&corpus);
    fs::remove_dir_all(&corpus).unwrap();
    generate(&tiny(7), &corpus).unwrap();
    assert!(!first.is_empty());
    assert!(first == files(&corpus), "the second corpus differs");
}

#[test]
fn another_seed_writes_another_corpus() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let corpus = dir.path().join("corpus");
    generate(&tiny(7), &corpus).unwrap();
    let seven = files(&corpus);
    fs::remove_dir_all(&corpus).unwrap();
    generate(&tiny(8), &corpus).unwrap();
    assert!(
        seven != files(&corpus),
        "seeds 7 and 8 wrote the same corpus"
    );
}

#[test]
fn the_corpus_holds_what_its_shape_asks_for() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let corpus = generate(&tiny(7), dir.path()).unwrap();
    assert_eq!(corpus.sessions(), 20);
    assert_eq!(corpus.files, 23);
    assert!(corpus.bytes >= 200_000, "{} bytes", corpus.bytes);
}
