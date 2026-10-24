//! Lists and searches a history with its index kept under a tempdir, and
//! picks sessions out of the `Listing` that gives, for the format tests,
//! and says where each adapter's committed fixtures are.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use yapkeeper::{DataDir, Listed, Listing, Query, Roots, search};

pub const CLAUDE_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-claude/tests/fixtures/claude"
);
pub const CODEX_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-codex/tests/fixtures/codex"
);
pub const PI_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-pi/tests/fixtures/pi"
);

/// Every provider's committed fixtures as its root.
pub fn fixture_roots() -> Roots {
    Roots {
        claude: PathBuf::from(CLAUDE_FIXTURES),
        codex: PathBuf::from(CODEX_FIXTURES),
        pi: PathBuf::from(PI_FIXTURES),
    }
}

/// The data directory the index and preferences are kept in under `tmp`.
pub fn data_dir(tmp: &Path) -> DataDir {
    DataDir::at(tmp.join("data"))
}

/// The session listed with `id`, panicking with every listed id when it's missing.
pub fn listed<'a>(listing: &'a Listing, id: &str) -> &'a Listed {
    listing
        .sessions
        .iter()
        .find(|listed| listed.session.id == id)
        .unwrap_or_else(|| panic!("session {id} is missing from {:?}", ids(listing)))
}

/// The ids of the sessions `text` finds under `roots`, in listing order,
/// with the index kept under `tmp`.
pub fn found(roots: &Roots, tmp: &Path, text: &str) -> Vec<String> {
    let query = Query::parse(text, &roots.providers());
    owned_ids(&search(roots, &data_dir(tmp), &query).unwrap())
}

/// As `found`, with tool-level text included.
pub fn found_with_tools(roots: &Roots, tmp: &Path, text: &str) -> Vec<String> {
    let query = Query::parse(text, &roots.providers()).including_tools(true);
    owned_ids(&search(roots, &data_dir(tmp), &query).unwrap())
}

fn owned_ids(listing: &Listing) -> Vec<String> {
    ids(listing).into_iter().map(str::to_string).collect()
}

/// The ids of `listing`'s sessions, in listing order.
pub fn ids(listing: &Listing) -> Vec<&str> {
    listing
        .sessions
        .iter()
        .map(|listed| listed.session.id.as_str())
        .collect()
}

pub fn timestamp(rfc3339: &str) -> SystemTime {
    SystemTime::from(rfc3339.parse::<jiff::Timestamp>().unwrap())
}
