//! Drives `yapkeeper::list` over the adapters' committed fixtures.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use yapkeeper::{Roots, list};

const CLAUDE_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-claude/tests/fixtures/claude"
);
const CODEX_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../yapkeeper-codex/tests/fixtures/codex"
);

#[test]
fn list_merges_every_provider_most_recent_first() {
    let listing = list(&Roots {
        claude: PathBuf::from(CLAUDE_FIXTURES),
        codex: PathBuf::from(CODEX_FIXTURES),
    });
    let sessions: Vec<(&str, &str)> = listing
        .sessions
        .iter()
        .map(|listed| (listed.session.provider, listed.session.id.as_str()))
        .collect();
    assert_eq!(
        sessions,
        [
            ("codex", "cccccccc-0000-7000-8000-00000000000c"),
            ("claude", "22222222-2222-4222-8222-222222222222"),
            ("codex", "aaaaaaaa-0000-7000-8000-00000000000a"),
            ("claude", "11111111-1111-4111-8111-111111111111"),
            ("claude", "33333333-3333-4333-8333-333333333333"),
            ("claude", "77777777-7777-4777-8777-777777777777"),
            ("codex", "bbbbbbbb-0000-7000-8000-00000000000b"),
        ]
    );
}
