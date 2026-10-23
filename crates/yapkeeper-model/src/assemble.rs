//! The fold rules that turn facts into sessions: which title wins, which
//! identity and Git count, which time is a session's activity, and how a
//! subagent chain reaches its session. The digest's stored layout stays in
//! here; callers only keep the text `encode` gives them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{Adapter, Fact, RecordedGit, Role, Session, ThreadKind, TitleKind};

/// Bump when `absorb` or `assemble` would give different results from the same facts.
pub const FOLD_VERSION: u32 = 1;

/// What one file's facts fold to so far: enough to build its thread's
/// session, without message text. Folding facts in several batches gives the
/// same digest as folding them in one.
#[derive(Debug, Default)]
pub struct SourceDigest {
    fold: Fold,
}

// Kept apart from `SourceDigest` so the stored layout is never part of the
// public interface.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Fold {
    /// `None` until the file's first `Identity` fact.
    identity: Option<Identity>,
    working_dirs: Vec<PathBuf>,
    latest_activity: Option<SystemTime>,
    modified: Option<SystemTime>,
    first_prompt: Option<String>,
    own_titles: Titles,
    /// Titles this file gives other threads, by thread id.
    titles_for: BTreeMap<String, Titles>,
    recap: Option<String>,
    linked_prs: Vec<u64>,
    git: Option<Git>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Identity {
    id: Option<String>,
    kind: Kind,
}

#[derive(Debug, Serialize, Deserialize)]
enum Kind {
    Session,
    Subagent { parent: Option<String> },
}

/// The latest title of each kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Titles {
    named: Option<String>,
    generated: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Git {
    branch: Option<String>,
    repository_url: Option<String>,
}

impl SourceDigest {
    /// Folds in the next facts from the same file, in file order. `modified`
    /// is the file's mtime now, used when no fact records activity.
    pub fn absorb(&mut self, facts: &[Fact], modified: SystemTime) {
        let fold = &mut self.fold;
        fold.modified = Some(modified);
        for fact in facts {
            match fact {
                Fact::Identity { id, kind } => {
                    if fold.identity.is_none() {
                        fold.identity = Some(Identity {
                            id: id.clone(),
                            kind: match kind {
                                ThreadKind::Session => Kind::Session,
                                ThreadKind::Subagent { parent } => Kind::Subagent {
                                    parent: parent.clone(),
                                },
                            },
                        });
                    }
                }
                Fact::WorkingDir(dir) => {
                    if !fold.working_dirs.contains(dir) {
                        fold.working_dirs.push(dir.clone());
                    }
                }
                Fact::Activity(time) => {
                    fold.latest_activity = fold.latest_activity.max(Some(*time));
                }
                Fact::Message(message) => {
                    if fold.first_prompt.is_none() && message.role == Role::User {
                        fold.first_prompt = Some(message.text.clone());
                    }
                }
                Fact::Title { thread, kind, text } => {
                    let titles = match thread {
                        None => &mut fold.own_titles,
                        Some(thread) => fold.titles_for.entry(thread.clone()).or_default(),
                    };
                    let slot = match kind {
                        TitleKind::Named => &mut titles.named,
                        TitleKind::Generated => &mut titles.generated,
                    };
                    *slot = Some(text.clone());
                }
                Fact::Recap(text) => fold.recap = Some(text.clone()),
                Fact::LinkedPr(number) => {
                    if !fold.linked_prs.contains(number) {
                        fold.linked_prs.push(*number);
                    }
                }
                Fact::Git(git) => {
                    if fold.git.is_none() {
                        fold.git = Some(Git {
                            branch: git.branch.clone(),
                            repository_url: git.repository_url.clone(),
                        });
                    }
                }
            }
        }
    }

    /// The thread this file records, once an `Identity` fact has named one.
    pub fn thread(&self) -> Option<&str> {
        self.fold.identity.as_ref()?.id.as_deref()
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(&self.fold).expect("a digest is always representable as JSON")
    }

    /// `None` when `text` is not something `encode` returned.
    pub fn decode(text: &str) -> Option<SourceDigest> {
        Some(SourceDigest {
            fold: serde_json::from_str(text).ok()?,
        })
    }
}

/// One provider's sessions, and which session each thread belongs to.
#[derive(Debug, Default)]
pub struct Assembled {
    pub sessions: Vec<Session>,
    /// Thread id → the id of the session it belongs to (a session maps to itself).
    pub session_of: HashMap<String, String>,
}

/// One provider's sessions from its files' digests, given in the byte order
/// of their source paths. Threads with no prompt or no working directory, subagents whose
/// chain reaches no session, and files without an identity are left out.
pub fn assemble(adapter: &dyn Adapter, digests: &[SourceDigest]) -> Assembled {
    // A title aimed at a thread from another file, such as Codex's session
    // index, beats one of the same kind in the thread's own file; among
    // other files, the later path wins.
    let mut titles_for: HashMap<&str, Titles> = HashMap::new();
    for digest in digests {
        for (thread, titles) in &digest.fold.titles_for {
            let kept = titles_for.entry(thread).or_default();
            kept.named = titles.named.clone().or(kept.named.take());
            kept.generated = titles.generated.clone().or(kept.generated.take());
        }
    }

    let mut assembled = Assembled::default();
    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    let mut subagents = Vec::new();
    for digest in digests {
        let fold = &digest.fold;
        let Some(Identity { id: Some(id), kind }) = &fold.identity else {
            continue;
        };
        match kind {
            Kind::Subagent { parent } => {
                if let Some(parent) = parent {
                    parent_of.insert(id, parent);
                }
                subagents.push(id.as_str());
            }
            Kind::Session => {
                let (Some(working_dir), Some(first_prompt), Some(last_active)) = (
                    fold.working_dirs.first(),
                    &fold.first_prompt,
                    fold.latest_activity.or(fold.modified),
                ) else {
                    continue;
                };
                let other = titles_for.get(id.as_str()).cloned().unwrap_or_default();
                let title = other
                    .named
                    .or_else(|| fold.own_titles.named.clone())
                    .or(other.generated)
                    .or_else(|| fold.own_titles.generated.clone())
                    .unwrap_or_else(|| first_prompt.clone());
                let recorded_git = fold
                    .git
                    .as_ref()
                    .map(|git| RecordedGit {
                        branch: git.branch.clone(),
                        repository_url: git.repository_url.clone(),
                    })
                    .unwrap_or_default();
                assembled.session_of.insert(id.clone(), id.clone());
                assembled.sessions.push(Session {
                    provider: adapter.provider(),
                    id: id.clone(),
                    title,
                    working_dir: working_dir.clone(),
                    other_working_dirs: fold.working_dirs[1..].to_vec(),
                    last_active,
                    recorded_git,
                    recap: fold.recap.clone(),
                    linked_prs: fold.linked_prs.clone(),
                    subagent_count: 0,
                    resume: adapter.resume(id, working_dir),
                });
            }
        }
    }

    let session_ids: HashSet<String> = assembled.session_of.keys().cloned().collect();
    for subagent in subagents {
        let mut thread = parent_of.get(subagent).copied();
        // Bounded so a cycle in corrupt records can't loop forever.
        for _ in 0..=parent_of.len() {
            let Some(id) = thread else { break };
            if session_ids.contains(id) {
                assembled
                    .session_of
                    .insert(subagent.to_string(), id.to_string());
                if let Some(session) = assembled.sessions.iter_mut().find(|s| s.id == id) {
                    session.subagent_count += 1;
                }
                break;
            }
            thread = parent_of.get(id).copied();
        }
    }
    assembled
}
