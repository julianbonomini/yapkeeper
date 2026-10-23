//! Sessions from every provider, merged and ordered. Each provider's format
//! stays inside its adapter; this crate only knows where each one is rooted.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use yapkeeper_claude::ClaudeCode;
use yapkeeper_codex::Codex;

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
    pub sessions: Vec<Session>,
    pub malformed_lines: usize,
    /// Each provider's directory looked in, for the empty-state message.
    pub searched: Vec<PathBuf>,
}

/// Sessions from every provider, most recent `last_active` first. Ties break
/// by provider, then id, so the order is stable.
pub fn list(roots: &Roots) -> Listing {
    let adapters: [&dyn Adapter; 2] = [
        &ClaudeCode::at(roots.claude.clone()),
        &Codex::at(roots.codex.clone()),
    ];
    let mut listing = Listing {
        sessions: Vec::new(),
        malformed_lines: 0,
        searched: Vec::new(),
    };
    for adapter in adapters {
        let scan = adapter.scan();
        listing.sessions.extend(scan.sessions);
        listing.malformed_lines += scan.malformed_lines;
        listing.searched.push(scan.searched);
    }
    listing.sessions.sort_by(|a, b| {
        b.last_active
            .cmp(&a.last_active)
            .then_with(|| a.provider.cmp(b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });
    listing
}
