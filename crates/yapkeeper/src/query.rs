//! The query language: how what the person typed becomes search words, and
//! what it means for text to match one. It hides word splitting and what
//! "ignoring case" means; the index only stores and looks up folded text.

/// What the person typed, read as search words: each whitespace-separated
/// piece is a word, and a session must contain every one, in its title,
/// recap or messages, as a substring. A word with an uppercase letter
/// matches case; any other word ignores case, letter by letter. Nothing in
/// it is special yet; any text parses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    words: Vec<Word>,
    tools: bool,
}

impl Query {
    /// Never fails; empty or all-space text is the query every session matches.
    pub fn parse(text: &str) -> Query {
        Query {
            words: text.split_whitespace().map(Word::new).collect(),
            tools: false,
        }
    }

    /// The same query, also searching tool calls, tool output and subagent
    /// transcripts when `tools` is true.
    pub fn including_tools(self, tools: bool) -> Query {
        Query { tools, ..self }
    }

    pub(crate) fn words(&self) -> &[Word] {
        &self.words
    }

    pub(crate) fn tools(&self) -> bool {
        self.tools
    }
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
