//! Where a session ran, read from the disk each time it is asked: the Git
//! repo and worktree its directory belongs to, whether that directory is
//! still there, and whether it sits in a system temp folder. Git's on-disk
//! layouts and the guesses made for missing directories stay in here.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path};

use yapkeeper_model::RecordedGit;

// Paths are compared as recorded, not canonicalized, so macOS's
// `/var/folders` and its real path `/private/var/folders` are both listed.
const TEMP_FOLDERS: [&str; 4] = [
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];

/// Where a session ran and whether it can resume there, read from the disk now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub place: Place,
    pub availability: Availability,
    /// Under one of the system temp folders.
    pub temp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A repo's main checkout, named after its folder.
    MainCheckout { project: String },
    /// A linked worktree, grouped under its main checkout's project.
    Worktree { project: String, worktree: String },
    /// Not in a Git repo, or nothing could be inferred; show the working directory.
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// The directory no longer exists.
    Orphaned,
    /// The directory exists but can't be inspected, such as when permission is denied.
    Unavailable,
}

impl Location {
    /// The location of `working_dir` as the filesystem shows it now. When the
    /// directory is missing or can't be inspected, project and worktree are
    /// inferred from `recorded` and the path, falling back to `Folder`.
    /// Never fails.
    pub fn of(working_dir: &Path, recorded: &RecordedGit) -> Location {
        let mut availability = match fs::metadata(working_dir) {
            Ok(metadata) if metadata.is_dir() => Availability::Available,
            Ok(_) => Availability::Orphaned,
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                Availability::Orphaned
            }
            Err(_) => Availability::Unavailable,
        };
        let place = if availability == Availability::Available {
            match repo_place(working_dir) {
                Ok(place) => place,
                Err(Uninspectable) => {
                    availability = Availability::Unavailable;
                    inferred_place(working_dir, recorded)
                }
            }
        } else {
            inferred_place(working_dir, recorded)
        };
        Location {
            place,
            availability,
            temp: TEMP_FOLDERS
                .iter()
                .any(|folder| working_dir.starts_with(folder)),
        }
    }
}

/// A best guess at the place of a `working_dir` that can't be inspected,
/// from what the provider recorded and the path, falling back to `Folder`.
fn inferred_place(working_dir: &Path, recorded: &RecordedGit) -> Place {
    // The project comes from the first that gives one: the recorded remote,
    // a live repo above the directory, then the path layout. A live
    // worktree found above is kept whole, as the disk knows best.
    let layout = worktree_layout(working_dir);
    let project = match recorded.repository_url.as_deref().and_then(repo_name) {
        Some(project) => project,
        None => match nearest_live_place(working_dir) {
            Some(Place::MainCheckout { project }) => project,
            Some(worktree @ Place::Worktree { .. }) => return worktree,
            Some(Place::Folder) | None => {
                return match layout {
                    Some(layout) => Place::Worktree {
                        project: layout.project,
                        worktree: layout.worktree,
                    },
                    None => Place::Folder,
                };
            }
        },
    };
    if let Some(layout) = layout {
        return Place::Worktree {
            project,
            worktree: layout.worktree,
        };
    }
    // Worktrees are named after their folder, so a branch only names one
    // when its last segment is that folder's name.
    let folder = folder_name(working_dir);
    let branch_names_folder = recorded
        .branch
        .as_deref()
        .and_then(|branch| branch.rsplit('/').next())
        .is_some_and(|last| last == folder);
    if branch_names_folder && folder != project {
        Place::Worktree {
            project,
            worktree: folder,
        }
    } else {
        Place::MainCheckout { project }
    }
}

/// Project and worktree names read from a path laid out the way worktree
/// tools lay out their folders.
struct WorktreeLayout {
    project: String,
    worktree: String,
}

/// The layout `dir` or its nearest ancestor follows: `<project>.wt/<worktree>`,
/// `<project>/.claude/worktrees/<worktree>`, a tool's home such as
/// `~/.vago/worktrees/<project>/<worktree>`, or
/// `<project>/worktrees/<worktree>` and `<project>/.worktrees/<worktree>`.
fn worktree_layout(dir: &Path) -> Option<WorktreeLayout> {
    let names: Vec<String> = dir
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    (1..=names.len()).rev().find_map(|end| {
        let (project, worktree) = match &names[..end] {
            [.., folder, worktree] if folder.len() > ".wt".len() && folder.ends_with(".wt") => {
                (folder.strip_suffix(".wt")?, worktree)
            }
            [.., project, dot_claude, worktrees, worktree]
                if dot_claude == ".claude" && worktrees == "worktrees" =>
            {
                (project.as_str(), worktree)
            }
            // `.claude` is excluded so a folder inside a Claude Code worktree
            // isn't read as `.claude/worktrees/<project>/<worktree>`.
            [.., tool_home, worktrees, project, worktree]
                if tool_home.starts_with('.')
                    && tool_home != ".claude"
                    && worktrees == "worktrees" =>
            {
                (project.as_str(), worktree)
            }
            [.., project, worktrees, worktree]
                if (worktrees == "worktrees" || worktrees == ".worktrees")
                    && !project.starts_with('.') =>
            {
                (project.as_str(), worktree)
            }
            _ => return None,
        };
        Some(WorktreeLayout {
            project: project.to_string(),
            worktree: worktree.clone(),
        })
    })
}

/// The place of the nearest ancestor of `dir` that is still a directory, or
/// `None` when that one can't be inspected either.
fn nearest_live_place(dir: &Path) -> Option<Place> {
    let live = dir
        .ancestors()
        .find(|ancestor| fs::metadata(ancestor).is_ok_and(|metadata| metadata.is_dir()))?;
    repo_place(live).ok()
}

/// The repo a remote URL such as `git@example.com:acme/acme-api.git` names.
fn repo_name(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

/// A `.git` that exists but couldn't be read, so which repo a directory
/// belongs to is unknown.
struct Uninspectable;

/// The repo `dir` sits in, found at the nearest `.git` above it: a `.git`
/// directory marks a main checkout and a `.git` file a linked worktree.
fn repo_place(dir: &Path) -> Result<Place, Uninspectable> {
    for checkout in dir.ancestors() {
        let dot_git = checkout.join(".git");
        let metadata = match fs::metadata(&dot_git) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Err(Uninspectable),
        };
        if metadata.is_dir() {
            return Ok(Place::MainCheckout {
                project: folder_name(checkout),
            });
        }
        let contents = fs::read_to_string(&dot_git).map_err(|_| Uninspectable)?;
        return Ok(linked_place(checkout, &contents));
    }
    Ok(Place::Folder)
}

/// The place of a checkout whose `.git` is a file holding `contents`. A
/// linked worktree's `gitdir:` points into `<common dir>/worktrees/`; any
/// other shape, such as a submodule's, counts as a checkout of its own.
fn linked_place(checkout: &Path, contents: &str) -> Place {
    let own = Place::MainCheckout {
        project: folder_name(checkout),
    };
    let Some(gitdir) = contents
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
    else {
        return own;
    };
    let gitdir = checkout.join(gitdir.trim());
    let Some(worktrees) = gitdir.parent() else {
        return own;
    };
    let Some(common_dir) = worktrees.parent() else {
        return own;
    };
    if worktrees.file_name().is_none_or(|name| name != "worktrees") {
        return own;
    }
    // A bare repo's common dir is the repo itself, often named `<project>.git`.
    let project = match common_dir.parent() {
        Some(main_checkout) if common_dir.file_name().is_some_and(|name| name == ".git") => {
            folder_name(main_checkout)
        }
        _ => {
            let name = folder_name(common_dir);
            name.strip_suffix(".git")
                .map(str::to_string)
                .unwrap_or(name)
        }
    };
    Place::Worktree {
        project,
        worktree: folder_name(checkout),
    }
}

/// The last component of `path`, or the whole path when it has none, as `/` doesn't.
fn folder_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}
