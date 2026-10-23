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
//! case. While the last token is a filter being typed, it offers the ways to
//! finish it, each with how many sessions it lists.

use std::borrow::Borrow;
use std::ops::Range;
use std::path::Path;

use crate::{Availability, LeftOut, Listed, Location, Place, RecordedGit};

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
            State::Hidden => listed.hidden,
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
            let (negated, rest) = negation(&text[span.clone()]);
            let Parsed { kind, filter } = parse_rest(rest, negated, providers);
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

    /// The byte ranges of `text` that the query's search words match, by
    /// the same rule as the search, in order, with overlapping ranges
    /// merged; empty when no word matches. Each range covers whole
    /// characters. Excluded words are never highlighted.
    pub fn highlights(&self, text: &str) -> Vec<Range<usize>> {
        let folded = Folded::of(text);
        let mut found: Vec<Range<usize>> = self
            .words
            .iter()
            .flat_map(|word| word.find_in(text, &folded))
            .collect();
        found.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in found {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
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
    /// ones unless it has `is:hidden`; returns which of the two defaults
    /// left out a session every filter admits.
    pub(crate) fn select<L: Borrow<Listed>>(&self, sessions: &mut Vec<L>) -> LeftOut {
        let asked = |state| {
            self.conditions.iter().any(|condition| {
                !condition.negated && condition.filter == Filter::State(Some(state))
            })
        };
        let (temp_asked, hidden_asked) = (asked(State::Temp), asked(State::Hidden));
        let mut left_out = LeftOut::default();
        sessions.retain(|listed| {
            let listed = listed.borrow();
            let admitted = self
                .conditions
                .iter()
                .all(|condition| condition.filter.admits(listed) != condition.negated);
            let temp = !temp_asked && State::Temp.of(listed);
            let hidden = !hidden_asked && State::Hidden.of(listed);
            left_out.temp |= admitted && temp;
            left_out.hidden |= admitted && hidden;
            admitted && !temp && !hidden
        });
        sessions.sort_by(|a, b| {
            let (a, b) = (a.borrow(), b.borrow());
            State::Orphaned
                .of(a)
                .cmp(&State::Orphaned.of(b))
                .then_with(|| b.session.last_active.cmp(&a.session.last_active))
                .then_with(|| a.session.provider.cmp(b.session.provider))
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        left_out
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
            Filter::Location(name) => {
                let key = match &listed.location.place {
                    Place::Folder => None,
                    place => Some(location_name(place, &listed.session.working_dir)),
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
struct Parsed {
    kind: TokenKind,
    /// What the token asks of a session; `None` for a word or an
    /// incomplete token.
    filter: Option<Filter>,
}

/// What `rest` is read as. A bare provider or state name is a filter only
/// when `negated`; otherwise it is a word.
fn parse_rest(rest: &str, negated: bool, providers: &[&'static str]) -> Parsed {
    let provider = |name: &str| {
        let name = fold(name);
        providers.iter().copied().find(|provider| *provider == name)
    };
    let unfiltered = |kind| Parsed { kind, filter: None };
    let filter = match sigil(rest) {
        Some((_, "")) => return unfiltered(TokenKind::Incomplete),
        Some((Sigil::Location, name)) => Filter::Location(fold(name)),
        Some((Sigil::Provider, name)) => Filter::Provider(provider(name)),
        Some((Sigil::State, name)) => Filter::State(State::named(name)),
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
    Parsed {
        kind,
        filter: Some(filter),
    }
}

/// What a token names after its `!` or `not:`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sigil {
    /// `@`
    Location,
    /// `#`
    Provider,
    /// `is:`
    State,
}

/// The sigil `rest` starts with, and the name after it.
fn sigil(rest: &str) -> Option<(Sigil, &str)> {
    if let Some(name) = rest.strip_prefix('@') {
        Some((Sigil::Location, name))
    } else if let Some(name) = rest.strip_prefix('#') {
        Some((Sigil::Provider, name))
    } else {
        strip_prefix_ignoring_case(rest, "is:").map(|name| (Sigil::State, name))
    }
}

/// `text` split after its `!` or `not:`, as typed: whether there was one,
/// and the rest.
fn negation(text: &str) -> (bool, &str) {
    match text.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => match strip_prefix_ignoring_case(text, "not:") {
            Some(rest) => (true, rest),
            None => (false, text),
        },
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
        if self.matches_case() {
            text.contains(&self.text)
        } else {
            fold(text).contains(&self.key)
        }
    }

    /// Every range of `text`, whose fold is `folded`, that this word
    /// matches by the rule of `matches`, not overlapping one another.
    fn find_in(&self, text: &str, folded: &Folded) -> Vec<Range<usize>> {
        if self.matches_case() {
            return text
                .match_indices(&self.text)
                .map(|(at, found)| at..at + found.len())
                .collect();
        }
        folded
            .text
            .match_indices(&self.key)
            .map(|(at, found)| folded.origin(at..at + found.len()))
            .collect()
    }

    /// Whether case counts: only when the word has an uppercase letter.
    fn matches_case(&self) -> bool {
        self.text.chars().any(char::is_uppercase)
    }
}

/// A text's `fold`, with where each folded character came from, so a match
/// in the fold can be shown in the original text.
struct Folded {
    text: String,
    /// For each original character, in order: where its fold starts in
    /// `text`, and its own byte range in the original.
    chars: Vec<(usize, Range<usize>)>,
}

impl Folded {
    fn of(original: &str) -> Folded {
        let mut text = String::new();
        let mut chars = Vec::new();
        for (at, c) in original.char_indices() {
            chars.push((text.len(), at..at + c.len_utf8()));
            text.extend(c.to_lowercase());
        }
        Folded { text, chars }
    }

    /// The original characters whose folds `range` of the fold touches.
    fn origin(&self, range: Range<usize>) -> Range<usize> {
        let containing = |at: usize| self.chars.partition_point(|(start, _)| *start <= at) - 1;
        let first = &self.chars[containing(range.start)].1;
        let last = &self.chars[containing(range.end - 1)].1;
        first.start..last.end
    }
}

/// `text` with case ignored: every character lowercased on its own, so the
/// fold of a whole text contains the fold of any part of it, and a word that
/// matches a text always has its key in the text's fold. The index stores
/// this fold, so changing it means bumping the index's schema version.
pub(crate) fn fold(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// The name `@` takes for a session at `place` in `working_dir`: the
/// project, `project:worktree`, or for a folder its path. `@project` finds
/// every worktree too, as a filter matches any part of the name.
fn location_name(place: &Place, working_dir: &Path) -> String {
    match place {
        Place::MainCheckout { project } => project.clone(),
        Place::Worktree { project, worktree } => format!("{project}:{worktree}"),
        Place::Folder => working_dir.to_string_lossy().into_owned(),
    }
}

/// The `@` token that finds sessions where `dir` is now: its project,
/// `project:worktree`, or its path, read from the disk now. `None` when that
/// name is empty or holds whitespace, which a token can't.
pub fn location_token(dir: &Path) -> Option<String> {
    let location = Location::of(dir, &RecordedGit::default());
    let name = location_name(&location.place, dir);
    fits_a_token(&name).then(|| format!("@{name}"))
}

/// Whether `name` can end a token: it isn't empty and holds no whitespace,
/// which would split it.
fn fits_a_token(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_whitespace)
}

/// One way to finish the filter being typed at the end of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The finished token as it would read, such as `is:pr` or `!#codex`,
    /// keeping its `!` or `not:` and sigil as typed.
    pub token: String,
    /// The whole query with the token finished and a space after it.
    pub query: String,
    /// How many sessions `token` alone lists, by the same defaults as a
    /// search.
    pub count: usize,
}

/// The ways to finish the last token of `text` when it starts with `@`,
/// `#`, `is:`, `!` or `not:` and nothing follows it, in this order: for `@`,
/// the locations of the sessions that token lists, most recent first, a
/// worktree's project before the worktree; for `#`, the `providers`
/// starting with what was typed after it; for `is:`, the states that do;
/// for a bare `!` or `not:`, those providers then those states. Empty when
/// `text` ends in whitespace or with a word, or when the only way to finish
/// the token is the token as typed.
pub(crate) fn completions(
    text: &str,
    providers: &[&'static str],
    sessions: &[Listed],
) -> Vec<Completion> {
    let Some(last) = pieces(text).pop().filter(|span| span.end == text.len()) else {
        return Vec::new();
    };
    let typed_token = &text[last.start..];
    let (negated, rest) = negation(typed_token);
    let named = sigil(rest);
    let typed = named.map_or(rest, |(_, name)| name);
    // What is kept as typed: everything up to the name being finished.
    let head = &text[..text.len() - typed.len()];
    let prefix = &text[last.start..head.len()];
    let starting = |name: &&str| fold(name).starts_with(&fold(typed));
    let matching_providers = || providers.iter().copied().filter(starting);
    let states = || State::ALL.into_iter().map(State::name).filter(starting);
    let names: Vec<String> = match named {
        Some((Sigil::Provider, _)) => matching_providers().map(String::from).collect(),
        Some((Sigil::State, _)) => states().map(String::from).collect(),
        Some((Sigil::Location, _)) => locations(&format!("@{typed}"), providers, sessions),
        None if negated => matching_providers()
            .chain(states())
            .map(String::from)
            .collect(),
        None => return Vec::new(),
    };
    let completions: Vec<Completion> = names
        .into_iter()
        .filter(|name| fits_a_token(name))
        .map(|name| {
            let token = format!("{prefix}{name}");
            Completion {
                count: listed_by(&token, providers, sessions).len(),
                query: format!("{head}{name} "),
                token,
            }
        })
        .collect();
    // A token typed out in full is searched as it stands, not offered back.
    match completions.as_slice() {
        [only] if fold(&only.token) == fold(typed_token) => Vec::new(),
        _ => completions,
    }
}

/// The names `@` takes for the sessions `location` lists, in its order, a
/// worktree's project before the worktree, each once.
fn locations(location: &str, providers: &[&'static str], sessions: &[Listed]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for listed in listed_by(location, providers, sessions) {
        let place = &listed.location.place;
        let project = match place {
            Place::Worktree { project, .. } => Some(project.clone()),
            _ => None,
        };
        for name in project
            .into_iter()
            .chain([location_name(place, &listed.session.working_dir)])
        {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// The `sessions` that `text`, holding no search words, lists, in a
/// search's order and by its defaults.
fn listed_by<'s>(
    text: &str,
    providers: &[&'static str],
    sessions: &'s [Listed],
) -> Vec<&'s Listed> {
    let mut found: Vec<&Listed> = sessions.iter().collect();
    Query::parse(text, providers).select(&mut found);
    found
}
