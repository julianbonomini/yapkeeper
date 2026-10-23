//! Drives `yapkeeper::list` over the Claude Code adapter's committed fixtures.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use yapkeeper::{Roots, list};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-claude/tests/fixtures/claude"
);

#[test]
fn list_returns_every_top_level_session_most_recent_first() {
    let listing = list(&Roots {
        claude: PathBuf::from(FIXTURES),
    });
    let ids: Vec<&str> = listing
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "22222222-2222-4222-8222-222222222222",
            "11111111-1111-4111-8111-111111111111",
            "33333333-3333-4333-8333-333333333333",
            "77777777-7777-4777-8777-777777777777",
        ]
    );
}
