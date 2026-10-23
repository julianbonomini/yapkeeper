//! Sessions from every provider, merged and ordered, each with where it ran.
//! Each provider's format stays inside its adapter; this crate knows where
//! each one is rooted and reads each session's location from the disk.

#![forbid(unsafe_code)]

mod location;

use std::path::PathBuf;

use yapkeeper_claude::ClaudeCode;
use yapkeeper_codex::Codex;

pub use location::*;
pub use yapkeeper_model::*;

/// Where each provider keeps its history.
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
}

impl Roots {
    /// The roots this process's environment names. Fails only when no home
    /// directory can be found for a provider that needs one.
    pub fn from_env() -> Result<Roots, MissingHome> {
        Ok(Roots {
            claude: yapkeeper_claude::config_dir()?,
            codex: yapkeeper_codex::home_dir()?,
        })
    }
}

/// Sessions from every provider, with what was skipped and where it looked.
pub struct Listing {
    /// Most recent first.
    pub sessions: Vec<Listed>,
    pub malformed_lines: usize,
    /// Each provider's directory looked in, for the empty-state message.
    pub searched: Vec<PathBuf>,
}

/// A session with its location as of this listing.
pub struct Listed {
    pub session: Session,
    pub location: Location,
}

/// Sessions from every provider, most recent `last_active` first, each with
/// its location as the disk shows it now. Ties break by provider, then id,
/// so the order is stable.
pub fn list(roots: &Roots) -> Listing {
    let adapters: [&dyn Adapter; 2] = [
        &ClaudeCode::at(roots.claude.clone()),
        &Codex::at(roots.codex.clone()),
    ];
    let mut sessions = Vec::new();
    let mut malformed_lines = 0;
    let mut searched = Vec::new();
    for adapter in adapters {
        let scan = adapter.scan();
        sessions.extend(scan.sessions);
        malformed_lines += scan.malformed_lines;
        searched.push(scan.searched);
    }
    sessions.sort_by(|a, b| {
        b.last_active
            .cmp(&a.last_active)
            .then_with(|| a.provider.cmp(b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });
    Listing {
        sessions: sessions
            .into_iter()
            .map(|session| Listed {
                location: Location::of(&session.working_dir, &session.recorded_git),
                session,
            })
            .collect(),
        malformed_lines,
        searched,
    }
}
