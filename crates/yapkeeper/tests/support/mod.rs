//! Real Git repos and locked directories for the location tests, built in the
//! system temp folder, which has no `.git` above it, so the location walk
//! can't climb into this repository.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let repo = parent.join("acme-api");
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
