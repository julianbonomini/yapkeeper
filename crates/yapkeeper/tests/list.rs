//! Drives `yapkeeper::list` over the adapters' committed fixtures.

#![forbid(unsafe_code)]

use yapkeeper::list;

mod listing;

use listing::{data_dir, fixture_roots};

#[test]
fn list_merges_every_provider_most_recent_first() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = list(&fixture_roots(), &data_dir(tmp.path())).unwrap();
    let sessions: Vec<(&str, &str)> = listing
        .sessions
        .iter()
        .map(|listed| (listed.session.provider, listed.session.id.as_str()))
        .collect();
    assert_eq!(
        sessions,
        [
            ("pi", "01b00000-0000-7000-8000-00000000000b"),
            ("codex", "cccccccc-0000-7000-8000-00000000000c"),
            ("pi", "01a00000-0000-7000-8000-00000000000a"),
            ("claude", "22222222-2222-4222-8222-222222222222"),
            ("codex", "aaaaaaaa-0000-7000-8000-00000000000a"),
            ("claude", "11111111-1111-4111-8111-111111111111"),
            ("claude", "33333333-3333-4333-8333-333333333333"),
            ("claude", "77777777-7777-4777-8777-777777777777"),
            ("codex", "bbbbbbbb-0000-7000-8000-00000000000b"),
        ]
    );
}
