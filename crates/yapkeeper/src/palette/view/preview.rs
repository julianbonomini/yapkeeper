//! The preview of the selected session: what it is, where it ran and
//! whether it can resume, then its recap and what was said in it. A section
//! the session lacks is left out with its blank line. Text wraps by
//! terminal cells, and what the query's words match is highlighted.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Theme, icon};
use crate::line::{age, directory, fit, fit_left, one_line, without_controls, wrap};
use crate::palette::Palette;
use crate::{Availability, Message, Place, Query, Role};

const TITLE_LINES: usize = 3;
const META_LINES: usize = 2;
const RECAP_LINES: usize = 4;
/// Lines for each message in every section after the recap.
const MESSAGE_LINES: usize = 3;
const MATCHES: usize = 3;

/// Draws the selected session's preview into `area`, after a rule on its
/// left; only the rule when nothing is selected.
pub(super) fn render(palette: &Palette, theme: &Theme, frame: &mut Frame, area: Rect) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(theme.dim)
        .padding(Padding::horizontal(1));
    let cells = usize::from(block.inner(area).width);
    let lines = lines(palette, theme, cells);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Every line of the selected session's preview, `cells` wide at most;
/// none when nothing is selected.
fn lines(palette: &Palette, theme: &Theme, cells: usize) -> Vec<Line<'static>> {
    let Some(listed) = palette.selected() else {
        return Vec::new();
    };
    let session = &listed.session;
    let query = &palette.parsed;
    let text = Text {
        query,
        theme,
        cells,
        provider: session.provider,
    };
    let mut lines = text.paragraph(&session.title, "", TITLE_LINES, theme.title);

    let conversation = &palette.conversation;
    let mut meta = vec![
        session.provider.to_string(),
        format!("{} ago", age(session.last_active, SystemTime::now())),
        counted(conversation.len(), "message"),
    ];
    if session.subagent_count > 0 {
        meta.push(counted(session.subagent_count, "subagent"));
    }
    if !session.linked_prs.is_empty() {
        let numbers: Vec<String> = session.linked_prs.iter().map(|n| format!("#{n}")).collect();
        let label = if numbers.len() == 1 { "PR" } else { "PRs" };
        meta.push(format!("{label} {}", numbers.join(", ")));
    }
    // Facts about the session, not what was said in it, so a word that
    // matches them isn't highlighted.
    let unsearched = Text {
        query: &Query::default(),
        ..text
    };
    lines.extend(unsearched.paragraph(&meta.join(" · "), "", META_LINES, theme.dim));
    let icon = icon(listed);
    let dir = directory(&session.working_dir, palette.home.as_deref());
    let dir = fit_left(&dir, cells.saturating_sub(icon.width() + 1));
    lines.push(Line::styled(format!("{icon} {dir}"), theme.dim));
    if let Place::Worktree { worktree, .. } = &listed.location.place {
        let label = "worktree ";
        let worktree = fit(
            &without_controls(worktree),
            cells.saturating_sub(label.width()),
        );
        lines.push(Line::styled(format!("{label}{worktree}"), theme.dim));
    }
    let availability = match listed.location.availability {
        Availability::Available => None,
        Availability::Orphaned => Some("orphaned · the folder no longer exists"),
        Availability::Unavailable => Some("unavailable · the folder can't be inspected"),
    };
    if let Some(availability) = availability {
        lines.push(Line::styled(fit(availability, cells), theme.label));
    }

    if let Some(recap) = &session.recap {
        section(&mut lines, theme, "Recap");
        lines.extend(text.paragraph(recap, "", RECAP_LINES, Style::new()));
    }
    let matching: Vec<&Message> = conversation
        .iter()
        .filter(|message| !query.highlights(&message.text).is_empty())
        .take(MATCHES)
        .collect();
    if !matching.is_empty() {
        section(&mut lines, theme, "Matches");
        for message in matching {
            lines.extend(text.message(message, true));
        }
    }
    let opening = conversation
        .iter()
        .position(|message| message.role == Role::User);
    if let Some(opening) = opening {
        section(&mut lines, theme, "Opening");
        lines.extend(text.message(&conversation[opening], false));
    }
    // The latest prompt and every reply after it; the prompt is left out
    // when it is the opening one, already shown.
    let latest = conversation
        .iter()
        .rposition(|message| message.role == Role::User)
        .map(|latest| match Some(latest) == opening {
            true => &conversation[latest + 1..],
            false => &conversation[latest..],
        })
        .unwrap_or_default();
    if !latest.is_empty() {
        section(&mut lines, theme, "Latest");
        for message in latest {
            lines.extend(text.message(message, false));
        }
    }
    lines
}

/// A blank line, then a section's heading.
fn section(lines: &mut Vec<Line<'static>>, theme: &Theme, heading: &'static str) {
    lines.push(Line::raw(""));
    lines.push(Line::styled(heading, theme.accent));
}

/// `1 message`, `2 messages`.
fn counted(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// How one session's text is drawn, `cells` wide, with what the query
/// matches highlighted.
struct Text<'a> {
    query: &'a Query,
    theme: &'a Theme,
    cells: usize,
    /// Such as `claude`, the label of the assistant's messages.
    provider: &'static str,
}

impl Text<'_> {
    /// `message` after its speaker's label, in `MESSAGE_LINES` lines at
    /// most; from shortly before the first match when `around_match`.
    fn message(&self, message: &Message, around_match: bool) -> Vec<Line<'static>> {
        let speaker = match message.role {
            Role::User => "you",
            _ => self.provider,
        };
        // One column of labels, as wide as the widest one.
        let label_cells = self.provider.width().max("you".width()) + 1;
        let label = format!("{speaker:<label_cells$}");
        let mut text = one_line(&message.text);
        let first_match = self.query.highlights(&text).first().map(|hit| hit.start);
        if let Some(start) = first_match.filter(|_| around_match) {
            // About half a line before the match, so it shows in a long message.
            let before = &text[..start];
            let room = self.cells.saturating_sub(label_cells) / 2;
            if before.width() > room {
                text = format!("{}{}", fit_left(before, room), &text[start..]);
            }
        }
        self.paragraph(&text, &label, MESSAGE_LINES, Style::new())
    }

    /// `text` on one line wrapped into at most `max_lines` lines in `style`,
    /// what the query matches highlighted, ending in `…` when cut. `label`
    /// starts the first line, in its own column.
    fn paragraph(
        &self,
        text: &str,
        label: &str,
        max_lines: usize,
        style: Style,
    ) -> Vec<Line<'static>> {
        let text = one_line(text);
        let label_cells = label.width();
        let wrapped = wrap(&text, self.cells.saturating_sub(label_cells), max_lines);
        let highlights = self.query.highlights(&text);
        let highlighted = style.patch(self.theme.highlight);
        let last = wrapped.lines.len().saturating_sub(1);
        let mut lines = Vec::new();
        for (n, range) in wrapped.lines.into_iter().enumerate() {
            let lead = match n {
                0 => label.to_string(),
                _ => " ".repeat(label_cells),
            };
            let mut spans = vec![Span::styled(lead, self.theme.dim)];
            let mut at = range.start;
            for hit in &highlights {
                let (start, end) = (hit.start.max(range.start), hit.end.min(range.end));
                if start >= end {
                    continue;
                }
                spans.push(Span::styled(text[at..start].to_string(), style));
                spans.push(Span::styled(text[start..end].to_string(), highlighted));
                at = end;
            }
            spans.push(Span::styled(text[at..range.end].to_string(), style));
            if wrapped.cut && n == last {
                spans.push(Span::styled("…", style));
            }
            lines.push(Line::from(spans));
        }
        lines
    }
}
