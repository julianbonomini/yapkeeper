//! Width-aware text helpers. Widths are terminal cells, never bytes.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn width(s: &str) -> usize {
    s.width()
}

/// Tail-truncate to `w` cells with an ellipsis.
pub fn trunc(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// Head-truncate so the end of a path survives: `…/billing-retry`.
pub fn trunc_left(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        tail.push(c);
        used += cw;
    }
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

pub fn pad(s: &str, w: usize) -> String {
    let t = trunc(s, w);
    let fill = w.saturating_sub(t.width());
    format!("{t}{}", " ".repeat(fill))
}

pub fn pad_left(s: &str, w: usize) -> String {
    let t = trunc(s, w);
    let fill = w.saturating_sub(t.width());
    format!("{}{t}", " ".repeat(fill))
}

/// Greedy word wrap to `w` cells. Long words are hard-split.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(4);
    let mut lines = Vec::new();
    for para in s.lines() {
        let mut line = String::new();
        let mut lw = 0;
        for word in para.split(' ') {
            let ww = word.width();
            if lw > 0 && lw + 1 + ww > w {
                lines.push(std::mem::take(&mut line));
                lw = 0;
            }
            if ww > w {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if lw + cw > w {
                        lines.push(std::mem::take(&mut line));
                        lw = 0;
                    }
                    line.push(c);
                    lw += cw;
                }
                continue;
            }
            if lw > 0 {
                line.push(' ');
                lw += 1;
            }
            line.push_str(word);
            lw += ww;
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Smart-case term search: lowercase terms match case-insensitively.
pub fn find(hay: &str, term: &str) -> Option<usize> {
    if term.chars().any(|c| c.is_uppercase()) {
        hay.find(term)
    } else {
        // ASCII lowercasing keeps byte offsets stable.
        hay.to_ascii_lowercase().find(&term.to_ascii_lowercase())
    }
}

pub fn contains(hay: &str, term: &str) -> bool {
    find(hay, term).is_some()
}

/// Split `text` into spans, styling every occurrence of any term.
pub fn highlight<'a>(text: &str, terms: &[String], base: Style, hl: Style) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = terms
            .iter()
            .filter(|t| !t.is_empty())
            .filter_map(|t| find(rest, t).map(|i| (i, t.len())))
            .filter(|(i, n)| rest.is_char_boundary(*i) && rest.is_char_boundary(i + n))
            .min_by_key(|(i, _)| *i);
        match next {
            Some((i, n)) => {
                if i > 0 {
                    spans.push(Span::styled(rest[..i].to_string(), base));
                }
                spans.push(Span::styled(rest[i..i + n].to_string(), hl));
                rest = &rest[i + n..];
            }
            None => {
                spans.push(Span::styled(rest.to_string(), base));
                break;
            }
        }
    }
    spans
}

/// A one-line window of `text` around the first term hit.
pub fn snippet(text: &str, terms: &[String], w: usize) -> String {
    let flat = text.replace('\n', " ");
    let hit = terms.iter().filter_map(|t| find(&flat, t)).min();
    let start = match hit {
        Some(i) if i > 24 => {
            let mut s = i - 20;
            while !flat.is_char_boundary(s) {
                s -= 1;
            }
            s
        }
        _ => 0,
    };
    let body = &flat[start..];
    if start > 0 {
        trunc(&format!("…{body}"), w)
    } else {
        trunc(body, w)
    }
}

pub fn ago(minutes: u32) -> String {
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 14 => format!("{}d", m / (60 * 24)),
        m => format!("{}w", m / (60 * 24 * 7)),
    }
}
