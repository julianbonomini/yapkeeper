//! The location list `@` offers: how the sessions `@` lists become one row
//! per project, its worktrees under it, then one per plain folder. It owns
//! the grouping, the order, which rows count as missing and each row's path.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use super::{Ending, fits_a_token, listed_by, location_name};
use crate::{Availability, Listed, Place, SessionKey};

/// One row of the location list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    pub kind: LocationRowKind,
    /// Every session at this place is orphaned: its directory no longer exists.
    pub missing: bool,
}

/// What a row of the location list stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationRowKind {
    /// A repo's main checkout with its worktrees. `worktrees` counts the
    /// worktree rows listed under it; `checkout` is `None` when no listed
    /// session ran in the main checkout.
    Project {
        project: String,
        worktrees: usize,
        checkout: Option<PathBuf>,
    },
    /// A linked worktree, listed under its project's row.
    Worktree { worktree: String, checkout: PathBuf },
    /// A plain folder, or a place nothing could be inferred for.
    Folder { path: PathBuf },
}

/// The sessions `@` lists at one project, in a search's order.
struct Project<'s> {
    project: String,
    main: Vec<&'s Listed>,
    worktrees: Vec<Worktree<'s>>,
}

impl<'s> Project<'s> {
    /// Every session in the project, its main checkout's and its worktrees'.
    fn sessions(&self) -> Vec<&'s Listed> {
        let worktrees = self
            .worktrees
            .iter()
            .flat_map(|linked| &linked.spot.sessions);
        self.main.iter().chain(worktrees).copied().collect()
    }
}

/// The sessions `@` lists at one linked worktree, under the worktree's name.
struct Worktree<'s> {
    worktree: String,
    spot: Spot<'s>,
}

/// The sessions `@` lists at one worktree or folder, in a search's order;
/// never empty once grouping is done.
struct Spot<'s> {
    /// What follows `@` in the row's token.
    name: String,
    sessions: Vec<&'s Listed>,
}

impl Spot<'_> {
    fn new(name: String) -> Self {
        Spot {
            name,
            sessions: Vec::new(),
        }
    }
}

/// The location list's rows for `@typed`, in order: each project, then its
/// worktrees, then the plain folders; each group missing last, then most
/// recent first, ties by name. Rows, paths and `missing` come from the
/// sessions a bare `@` lists; `typed` only picks which rows are kept, and
/// while it is empty the missing ones are left out.
pub(super) fn rows(typed: &str, providers: &[&'static str], sessions: &[Listed]) -> Vec<Ending> {
    let mut projects: Vec<Project> = Vec::new();
    let mut folders: Vec<Spot> = Vec::new();
    for listed in listed_by("@", providers, sessions) {
        let place = &listed.location.place;
        let name = location_name(place, &listed.session.working_dir);
        let group = match place {
            Place::MainCheckout { project } => &mut project_named(&mut projects, project).main,
            Place::Worktree { project, worktree } => {
                let worktrees = &mut project_named(&mut projects, project).worktrees;
                let same = |linked: &Worktree| linked.spot.name == name;
                let linked = entry(worktrees, same, || Worktree {
                    worktree: worktree.clone(),
                    spot: Spot::new(name.clone()),
                });
                &mut linked.spot.sessions
            }
            Place::Folder => {
                let same = |spot: &Spot| spot.name == name;
                &mut entry(&mut folders, same, || Spot::new(name.clone())).sessions
            }
        };
        group.push(listed);
    }

    let shown: HashSet<SessionKey> = listed_by(&format!("@{typed}"), providers, sessions)
        .into_iter()
        .map(|listed| SessionKey::of(&listed.session))
        .collect();
    let kept = |name: &str, sessions: &[&Listed]| {
        fits_a_token(name)
            && sessions
                .iter()
                .any(|listed| shown.contains(&SessionKey::of(&listed.session)))
            && !(typed.is_empty() && missing(sessions))
    };

    projects.sort_by_cached_key(|project| (rank(&project.sessions()), project.project.clone()));
    let mut rows = Vec::new();
    for project in projects {
        let all = project.sessions();
        if !kept(&project.project, &all) {
            continue;
        }
        let mut worktrees: Vec<Worktree> = project
            .worktrees
            .into_iter()
            .filter(|linked| kept(&linked.spot.name, &linked.spot.sessions))
            .collect();
        worktrees
            .sort_by_cached_key(|linked| (rank(&linked.spot.sessions), linked.spot.name.clone()));
        rows.push(Ending {
            name: project.project.clone(),
            location: Some(LocationRow {
                kind: LocationRowKind::Project {
                    project: project.project,
                    worktrees: worktrees.len(),
                    checkout: most_recent_root(&project.main),
                },
                missing: missing(&all),
            }),
        });
        rows.extend(worktrees.into_iter().map(|linked| Ending {
            location: Some(LocationRow {
                kind: LocationRowKind::Worktree {
                    worktree: linked.worktree,
                    checkout:
                        most_recent_root(&linked.spot.sessions).expect("a spot is never empty"),
                },
                missing: missing(&linked.spot.sessions),
            }),
            name: linked.spot.name,
        }));
    }
    let mut folders: Vec<Spot> = folders
        .into_iter()
        .filter(|spot| kept(&spot.name, &spot.sessions))
        .collect();
    folders.sort_by_cached_key(|spot| (rank(&spot.sessions), spot.name.clone()));
    rows.extend(folders.into_iter().map(|spot| Ending {
        location: Some(LocationRow {
            kind: LocationRowKind::Folder {
                path: spot.sessions[0].session.working_dir.clone(),
            },
            missing: missing(&spot.sessions),
        }),
        name: spot.name,
    }));
    rows
}

/// The group for `project` in `projects`, added empty when there is none.
fn project_named<'p, 's>(projects: &'p mut Vec<Project<'s>>, project: &str) -> &'p mut Project<'s> {
    let same = |group: &Project| group.project == project;
    entry(projects, same, || Project {
        project: project.to_string(),
        main: Vec::new(),
        worktrees: Vec::new(),
    })
}

/// The first of `groups` that `same` picks, or a new one from `new` added
/// at the end.
fn entry<G>(groups: &mut Vec<G>, same: impl Fn(&G) -> bool, new: impl FnOnce() -> G) -> &mut G {
    let at = match groups.iter().position(same) {
        Some(at) => at,
        None => {
            groups.push(new());
            groups.len() - 1
        }
    };
    &mut groups[at]
}

/// Whether every one of `sessions` is orphaned.
fn missing(sessions: &[&Listed]) -> bool {
    sessions
        .iter()
        .all(|listed| listed.location.availability == Availability::Orphaned)
}

/// Where a group of `sessions` sorts: missing last, then by its most recent
/// session, newest first.
fn rank(sessions: &[&Listed]) -> (bool, Reverse<SystemTime>) {
    let latest = sessions
        .iter()
        .map(|listed| listed.session.last_active)
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    (missing(sessions), Reverse(latest))
}

/// The root of the most recent of `sessions`; `None` when there are none.
fn most_recent_root(sessions: &[&Listed]) -> Option<PathBuf> {
    sessions
        .iter()
        .max_by_key(|listed| listed.session.last_active)
        .map(|listed| listed.location.root.clone())
}
