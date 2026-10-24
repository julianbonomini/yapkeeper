//! Real Git repos and locked directories for the location tests. They are
//! built in the system temp folder, which has no `.git` above it, or in a
//! live folder under the build's target directory, where each repo's own
//! `.git` stops the location walk before it climbs into this repository.

// Each test crate compiles this module on its own and uses only some of it.
#![allow(dead_code)]

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use yapkeeper::{Location, RecordedGit};

/// A fresh tempdir, checked to sit under a system temp folder: sessions in
/// it are temp, so the tests find them with `is:temp`.
pub fn temp_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Location::of(dir.path(), &RecordedGit::default()).temp,
        "{} is not under a system temp folder; unset TMPDIR or point it at /tmp",
        dir.path().display()
    );
    dir
}

/// A fresh directory under the build's target directory, checked not to sit
/// under a system temp folder: sessions in it are listed by default.
pub fn live_dir() -> TempDir {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    assert!(
        !Location::of(dir.path(), &RecordedGit::default()).temp,
        "{} is under a system temp folder; point CARGO_TARGET_DIR elsewhere",
        dir.path().display()
    );
    dir
}

/// Runs `git` in `dir` with an identity of its own, so the tests don't depend
/// on the machine's Git configuration.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A main checkout at `<parent>/acme-api` with one commit and a `src` folder.
pub fn main_checkout(parent: &Path) -> PathBuf {
    checkout(parent, "acme-api")
}

/// A main checkout at `<parent>/<name>` with one commit and a `src` folder.
fn checkout(parent: &Path, name: &str) -> PathBuf {
    let repo = parent.join(name);
    fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    repo
}

/// A main checkout at `<parent>/acme-api` and a linked worktree of it at
/// `<parent>/acme-api.wt/billing-retry`.
pub fn repo_with_worktree(parent: &Path) -> (PathBuf, PathBuf) {
    let repo = main_checkout(parent);
    git(
        &repo,
        &["worktree", "add", "-q", "../acme-api.wt/billing-retry"],
    );
    (repo, parent.join("acme-api.wt/billing-retry"))
}

/// A main checkout at `<parent>/<name>` and a linked worktree of it at
/// `<parent>/<name>.wt/<worktree>` for each of `worktrees`, in order.
pub fn repo_with_worktrees(
    parent: &Path,
    name: &str,
    worktrees: &[&str],
) -> (PathBuf, Vec<PathBuf>) {
    let repo = checkout(parent, name);
    let linked = worktrees
        .iter()
        .map(|worktree| {
            let path = format!("../{name}.wt/{worktree}");
            git(&repo, &["worktree", "add", "-q", &path]);
            parent.join(format!("{name}.wt/{worktree}"))
        })
        .collect();
    (repo, linked)
}

/// Takes every permission away from a directory until dropped, so the
/// tempdir can be deleted even when an assertion fails.
#[cfg(unix)]
pub struct Locked<'a>(&'a Path);

#[cfg(unix)]
impl Locked<'_> {
    pub fn new(dir: &Path) -> Locked<'_> {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        Locked(dir)
    }

    /// False when running as root, which reads the directory anyway.
    pub fn denies_access(&self) -> bool {
        fs::read_dir(self.0).is_err()
    }
}

#[cfg(unix)]
impl Drop for Locked<'_> {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
    }
}
