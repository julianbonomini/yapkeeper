//! The query language: how what the person typed becomes tokens, search
//! words and filters, and what it means for text to match a word. It hides
//! the grammar and its vocabulary of states and what "ignoring case" means;
//! the index only stores and looks up folded text.
//!
//! A token is a run of non-whitespace, so a space never joins tokens or
//! forms a filter. `!` or `not:` negates the rest of the token, which is read
//! as `@where`, `#provider` or `is:state` when it starts with one, otherwise
//! as a bare provider name, then a bare state name, and otherwise as a word;
//! a second `!` is just text. Sigils and names ignore case; words keep smart
//! case.

use std::ops::Range;

use crate::{Availability, Listed, Place};

/// What the person typed, read as the query language. A session must pass
/// every filter and contain every search word, in its title, recap or
/// messages, as a substring. A word with an uppercase letter matches case;
/// any other word ignores case, letter by letter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    tokens: Vec<Token>,
    words: Vec<Word>,
    excluded_words: Vec<Word>,
    conditions: Vec<Condition>,
    tools: bool,
}

/// One whitespace-separated piece of a query and what it was read as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// True for a `!` or `not:` prefix.
    pub negated: bool,
    /// The token's byte range in the parsed text, including its `!` or `not:`.
    pub span: Range<usize>,
}

/// What a token was read as, so it can be coloured and completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Searched for in titles, recaps and messages.
    Word,
    /// `@where`.
    Location,
    /// `#name`, or a bare provider name after `!` or `not:`. `None` when
    /// `#name` names no known provider: it then matches no session, and
    /// excludes none when negated.
    Provider(Option<&'static str>),
    /// `is:name`, or a bare state name after `!` or `not:`. `None` when
    /// `is:name` names no state, matching and excluding as `Provider(None)`.
    State(Option<State>),
    /// A prefix with nothing after it (`@`, `#`, `is:`, `!`, `not:`), as
    /// while typing. It matches every session.
    Incomplete,
}

/// What `is:name` asks about a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Orphaned,
    Temp,
    Git,
    Folder,
    Pr,
    Recap,
    Unavailable,
    Hidden,
}

impl State {
    /// Every state, in the order the interface doc lists them.
    pub const ALL: [State; 8] = [
        State::Orphaned,
        State::Temp,
        State::Git,
        State::Folder,
        State::Pr,
        State::Recap,
        State::Unavailable,
        State::Hidden,
    ];

    /// The name `is:` takes, such as `"pr"`.
    pub fn name(self) -> &'static str {
        match self {
            State::Orphaned => "orphaned",
            State::Temp => "temp",
            State::Git => "git",
            State::Folder => "folder",
            State::Pr => "pr",
            State::Recap => "recap",
            State::Unavailable => "unavailable",
            State::Hidden => "hidden",
        }
    }

    /// Whether `listed` is in this state.
    fn of(self, listed: &Listed) -> bool {
        let location = &listed.location;
        match self {
            State::Orphaned => location.availability == Availability::Orphaned,
            State::Unavailable => location.availability == Availability::Unavailable,
            State::Temp => location.temp,
            State::Git => matches!(
                location.place,
                Place::MainCheckout { .. } | Place::Worktree { .. }
            ),
            State::Folder => location.place == Place::Folder,
            State::Pr => !listed.session.linked_prs.is_empty(),
            State::Recap => listed.session.recap.is_some(),
            // Until hides exist, no session is hidden.
            State::Hidden => false,
        }
    }

    fn named(name: &str) -> Option<State> {
        let name = fold(name);
        State::ALL.into_iter().find(|state| state.name() == name)
    }
}

impl Query {
    /// Reads `text` as the query language; `providers` are the names
    /// `#name`, `!name` and `not:name` may name. Never fails; empty or
    /// all-space text is `Query::default()`, which every session outside
    /// temp folders and not hidden matches.
    pub fn parse(text: &str, providers: &[&'static str]) -> Query {
        let mut query = Query::default();
        for span in pieces(text) {
            let piece = &text[span.clone()];
            let (negated, rest) = match piece.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => match strip_prefix_ignoring_case(piece, "not:") {
                    Some(rest) => (true, rest),
                    None => (false, piece),
                },
            };
            let Reading { kind, filter } = read(rest, negated, providers);
            match filter {
                Some(filter) => query.conditions.push(Condition { filter, negated }),
                None if kind != TokenKind::Word => {}
                None if negated => query.excluded_words.push(Word::new(rest)),
                None => query.words.push(Word::new(rest)),
            }
            query.tokens.push(Token {
                kind,
                negated,
                span,
            });
        }
        query
    }

    /// The same query, also searching tool calls, tool output and subagent
    /// transcripts when `tools` is true.
    pub fn including_tools(self, tools: bool) -> Query {
        Query { tools, ..self }
    }

    /// Each whitespace-separated piece of the parsed text, in order, with
    /// what it was read as.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub(crate) fn words(&self) -> &[Word] {
        &self.words
    }

    /// The words a session must not contain, from `!word` and `not:word`.
    pub(crate) fn excluded_words(&self) -> &[Word] {
        &self.excluded_words
    }

    pub(crate) fn tools(&self) -> bool {
        self.tools
    }

    /// Keeps the sessions every non-word filter and both defaults admit,
    /// then orders them: orphaned last, each group most recent
    /// `last_active` first, ties by provider then id. Temp sessions are
    /// left out unless the query has a non-negated `is:temp`, and hidden
    /// ones unless it has `is:hidden`.
    pub(crate) fn select(&self, sessions: &mut Vec<Listed>) {
        let temp_asked = self.conditions.iter().any(|condition| {
            !condition.negated && condition.filter == Filter::State(Some(State::Temp))
        });
        sessions.retain(|listed| {
            (temp_asked || !State::Temp.of(listed))
                && self
                    .conditions
                    .iter()
                    .all(|condition| condition.filter.admits(listed) != condition.negated)
        });
        sessions.sort_by(|a, b| {
            State::Orphaned
                .of(a)
                .cmp(&State::Orphaned.of(b))
                .then_with(|| b.session.last_active.cmp(&a.session.last_active))
                .then_with(|| a.session.provider.cmp(b.session.provider))
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
    }
}

/// A filter, possibly negated.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Condition {
    filter: Filter,
    negated: bool,
}

/// A token other than a word, resolved to what it asks of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Filter {
    /// `@where`, its name folded.
    Location(String),
    /// `None` names no provider, so it matches no session.
    Provider(Option<&'static str>),
    /// `None` names no state, so it matches no session.
    State(Option<State>),
}

impl Filter {
    fn admits(&self, listed: &Listed) -> bool {
        match self {
            // `project:worktree` for a worktree, so `@project` finds every
            // worktree too and `@project:worktree` only that one.
            Filter::Location(name) => {
                let key = match &listed.location.place {
                    Place::MainCheckout { project } => Some(project.clone()),
                    Place::Worktree { project, worktree } => Some(format!("{project}:{worktree}")),
                    Place::Folder => None,
                };
                key.is_some_and(|key| fold(&key).contains(name))
                    || fold(&listed.session.working_dir.to_string_lossy()).contains(name)
            }
            Filter::Provider(provider) => *provider == Some(listed.session.provider),
            Filter::State(state) => state.is_some_and(|state| state.of(listed)),
        }
    }
}

/// What the rest of a token, after any `!` or `not:`, was read as.
struct Reading {
    kind: TokenKind,
    /// What the token asks of a session; `None` for a word or an
    /// incomplete token.
    filter: Option<Filter>,
}

/// What `rest` is read as. A bare provider or state name is a filter only
/// when `negated`; otherwise it is a word.
fn read(rest: &str, negated: bool, providers: &[&'static str]) -> Reading {
    let provider = |name: &str| {
        let name = fold(name);
        providers.iter().copied().find(|provider| *provider == name)
    };
    // The name after a sigil, and the filter it makes.
    let sigil = if let Some(name) = rest.strip_prefix('@') {
        Some((name, Filter::Location(fold(name))))
    } else if let Some(name) = rest.strip_prefix('#') {
        Some((name, Filter::Provider(provider(name))))
    } else {
        strip_prefix_ignoring_case(rest, "is:")
            .map(|name| (name, Filter::State(State::named(name))))
    };
    let unfiltered = |kind| Reading { kind, filter: None };
    let filter = match sigil {
        Some(("", _)) => return unfiltered(TokenKind::Incomplete),
        Some((_, filter)) => filter,
        None if rest.is_empty() => return unfiltered(TokenKind::Incomplete),
        None if !negated => return unfiltered(TokenKind::Word),
        None => match (provider(rest), State::named(rest)) {
            (Some(provider), _) => Filter::Provider(Some(provider)),
            (None, Some(state)) => Filter::State(Some(state)),
            (None, None) => return unfiltered(TokenKind::Word),
        },
    };
    let kind = match filter {
        Filter::Location(_) => TokenKind::Location,
        Filter::Provider(provider) => TokenKind::Provider(provider),
        Filter::State(state) => TokenKind::State(state),
    };
    Reading {
        kind,
        filter: Some(filter),
    }
}

/// The byte range of each run of non-whitespace in `text`.
fn pieces(text: &str) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = None;
    // A trailing space ends the last piece.
    for (at, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_whitespace(), start) {
            (true, Some(from)) => {
                pieces.push(from..at);
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    pieces
}

/// `text` without `prefix`, which matches ASCII letters in any case.
fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// One search word, as typed and folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    text: String,
    key: String,
}

impl Word {
    fn new(text: &str) -> Word {
        Word {
            text: text.to_string(),
            key: fold(text),
        }
    }

    /// The word folded, as the index looks it up.
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// Whether `text` contains this word, ignoring case unless the word has
    /// an uppercase letter. Never true unless `fold(text)` contains `key()`.
    pub(crate) fn matches(&self, text: &str) -> bool {
        if self.text.chars().any(char::is_uppercase) {
            text.contains(&self.text)
        } else {
            fold(text).contains(&self.key)
        }
    }
}

/// `text` with case ignored: every character lowercased on its own, so the
/// fold of a whole text contains the fold of any part of it, and a word that
/// matches a text always has its key in the text's fold. The index stores
/// this fold, so changing it means bumping the index's schema version.
pub(crate) fn fold(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}
