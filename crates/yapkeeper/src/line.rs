//! How a session's fields read as one line of terminal text, for the CLI
//! and the palette alike: free of control characters, fitted to a number of
//! terminal cells rather than bytes, and the words used for its location,
//! its age and whether it can resume.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{Availability, Listed, Place, State};

/// Text on one line: control characters such as ESC are removed, then each
/// whitespace run, newlines included, becomes one space.
pub fn one_line(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| c.is_whitespace() || !c.is_control())
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` without control characters, so recorded text can't move the
/// cursor or restyle the terminal.
pub fn without_controls(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// `text` in at most `cells` terminal cells, ending in `…` when cut.
pub fn fit(text: &str, cells: usize) -> String {
    if text.width() <= cells {
        return text.to_string();
    }
    if cells == 0 {
        return String::new();
    }
    let kept: String = keep_within(text.chars(), cells - 1).into_iter().collect();
    format!("{kept}…")
}

/// `text` in at most `cells` terminal cells, keeping its end and starting
/// with `…` when cut, as a path whose last folders matter most.
pub fn fit_left(text: &str, cells: usize) -> String {
    if text.width() <= cells {
        return text.to_string();
    }
    if cells == 0 {
        return String::new();
    }
    let kept: String = keep_within(text.chars().rev(), cells - 1)
        .into_iter()
        .rev()
        .collect();
    format!("…{kept}")
}

/// `text`, on one line as `one_line` gives it, broken at spaces into lines
/// of at most `cells` terminal cells, as byte ranges of `text` without the
/// spaces they break at. A word wider than `cells` is split between
/// characters. When more than `max_lines` lines would be needed, `cut` is
/// true and the last range is shortened to leave a cell for `…`.
pub fn wrap(text: &str, cells: usize, max_lines: usize) -> Wrapped {
    let mut lines = Vec::new();
    if cells == 0 || max_lines == 0 {
        return Wrapped {
            lines,
            cut: !text.is_empty(),
        };
    }
    let mut start = 0;
    let mut used = 0;
    // Where the current line can break: the last space in it.
    let mut space = None;
    for (at, c) in text.char_indices() {
        let width = c.width().unwrap_or(0);
        if c == ' ' && used + width > cells {
            lines.push(start..at);
            (start, used, space) = (at + 1, 0, None);
            continue;
        }
        if used + width > cells && at > start {
            let end = space.unwrap_or(at);
            lines.push(start..end);
            start = if space.is_some() { end + 1 } else { end };
            used = text[start..at].width();
            space = None;
        }
        if c == ' ' {
            space = Some(at);
        }
        used += width;
    }
    if start < text.len() || lines.is_empty() {
        lines.push(start..text.len());
    }
    let cut = lines.len() > max_lines;
    if cut {
        lines.truncate(max_lines);
        let last = lines.last_mut().expect("max_lines is at least one");
        let kept = keep_within(text[last.clone()].chars(), cells - 1);
        let kept: usize = kept.iter().map(|c| c.len_utf8()).sum();
        last.end = last.start + text[last.start..last.start + kept].trim_end().len();
    }
    Wrapped { lines, cut }
}

/// What `wrap` made of a text.
pub struct Wrapped {
    /// Byte ranges of the wrapped text, one per line, in order.
    pub lines: Vec<Range<usize>>,
    /// True when text was left out after the last line.
    pub cut: bool,
}

/// The leading `chars` that fit in `cells` terminal cells.
fn keep_within(chars: impl Iterator<Item = char>, cells: usize) -> Vec<char> {
    let mut width = 0;
    chars
        .take_while(|c| {
            width += c.width().unwrap_or(0);
            width <= cells
        })
        .collect()
}

/// How long ago `then` was, in the largest unit that keeps the number
/// readable: `42m`, `5h`, `9d`, `30w`. A time in the future shows `0m`.
pub fn age(then: SystemTime, now: SystemTime) -> String {
    let minutes = now
        .duration_since(then)
        .map(|elapsed| elapsed.as_secs() / 60)
        .unwrap_or(0);
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 14 => format!("{}d", m / (60 * 24)),
        m => format!("{}w", m / (60 * 24 * 7)),
    }
}

/// The home folder `place` shows as `~`: `HOME`, unless it is unset or
/// empty.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `project`, `project › worktree`, or, when the session ran outside any
/// known repo, its working directory with a leading `home` shown as `~`.
/// Never holds a control character.
pub fn place(listed: &Listed, home: Option<&Path>) -> String {
    let text = match &listed.location.place {
        Place::MainCheckout { project } => project.clone(),
        Place::Worktree { project, worktree } => format!("{project} › {worktree}"),
        Place::Folder => return directory(&listed.session.working_dir, home),
    };
    without_controls(&text)
}

/// `dir` with a leading `home` shown as `~`. Never holds a control
/// character.
pub fn directory(dir: &Path, home: Option<&Path>) -> String {
    let text = match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    };
    without_controls(&text)
}

/// `orphaned`, `unavailable`, or `""` for a session that can resume: the
/// words `is:` takes, so the state survives `NO_COLOR` and `grep`.
pub fn availability_label(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "",
        Availability::Orphaned => State::Orphaned.name(),
        Availability::Unavailable => State::Unavailable.name(),
    }
}
