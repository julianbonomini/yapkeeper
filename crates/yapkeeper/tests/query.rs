//! Checks what `yapkeeper::Query::parse` reads each piece of a query as,
//! the way the palette will colour and complete it.

#![forbid(unsafe_code)]

use yapkeeper::{Query, State, TokenKind};

#[test]
fn every_token_reports_its_kind_whether_it_is_negated_and_its_span() {
    let text = "retry @acme-api:wt #codex is:pr !flaky not:orphaned !codex is pr @ #pi is:bogus";
    let query = Query::parse(text, &["claude", "codex"]);

    let read: Vec<(&str, TokenKind, bool)> = query
        .tokens()
        .iter()
        .map(|token| (&text[token.span.clone()], token.kind, token.negated))
        .collect();
    assert_eq!(
        read,
        [
            ("retry", TokenKind::Word, false),
            ("@acme-api:wt", TokenKind::Location, false),
            ("#codex", TokenKind::Provider(Some("codex")), false),
            ("is:pr", TokenKind::State(Some(State::Pr)), false),
            ("!flaky", TokenKind::Word, true),
            (
                "not:orphaned",
                TokenKind::State(Some(State::Orphaned)),
                true
            ),
            ("!codex", TokenKind::Provider(Some("codex")), true),
            ("is", TokenKind::Word, false),
            ("pr", TokenKind::Word, false),
            ("@", TokenKind::Incomplete, false),
            ("#pi", TokenKind::Provider(None), false),
            ("is:bogus", TokenKind::State(None), false),
        ]
    );
    let spans: Vec<_> = query
        .tokens()
        .iter()
        .map(|token| token.span.clone())
        .collect();
    assert_eq!(
        spans,
        [
            0..5,
            6..18,
            19..25,
            26..31,
            32..38,
            39..51,
            52..58,
            59..61,
            62..64,
            65..66,
            67..70,
            71..79
        ]
    );
}

#[test]
fn spans_are_byte_ranges_across_any_whitespace() {
    let text = " \u{3000}échec\t!Codex\n\nIS:PR ";
    let query = Query::parse(text, &["claude", "codex"]);

    let read: Vec<(&str, TokenKind, bool)> = query
        .tokens()
        .iter()
        .map(|token| (&text[token.span.clone()], token.kind, token.negated))
        .collect();
    assert_eq!(
        read,
        [
            ("échec", TokenKind::Word, false),
            ("!Codex", TokenKind::Provider(Some("codex")), true),
            ("IS:PR", TokenKind::State(Some(State::Pr)), false),
        ]
    );
}
