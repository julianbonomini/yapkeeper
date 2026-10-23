//! The preview of the selected session: what it is, where it ran and
//! whether it can resume, then its recap and what was said in it. A section
//! the session lacks is left out with its blank line. Text wraps by
//! terminal cells, and what the query's words match is highlighted.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Text, Theme, icon};
use crate::line::{age, directory, fit, fit_left, one_line, without_controls};
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
    let mut lines = text
        .paragraph(&session.title, "", TITLE_LINES, theme.title)
        .lines;

    // The preview shows what was said; tool calls are for the conversation.
    let conversation: Vec<&Message> = palette
        .conversation
        .iter()
        .filter(|message| message.role != Role::ToolCall)
        .collect();
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
    lines.extend(
        unsearched
            .paragraph(&meta.join(" · "), "", META_LINES, theme.dim)
            .lines,
    );
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
        lines.extend(text.paragraph(recap, "", RECAP_LINES, Style::new()).lines);
    }
    let matching: Vec<&Message> = conversation
        .iter()
        .copied()
        .filter(|message| !query.highlights(&message.text).is_empty())
        .take(MATCHES)
        .collect();
    if !matching.is_empty() {
        section(&mut lines, theme, "Matches");
        for message in matching {
            lines.extend(said(&text, message, true));
        }
    }
    let opening = conversation
        .iter()
        .position(|message| message.role == Role::User);
    if let Some(opening) = opening {
        section(&mut lines, theme, "Opening");
        lines.extend(said(&text, conversation[opening], false));
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
            lines.extend(said(&text, message, false));
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

/// What `message` says, after its speaker's label, in `MESSAGE_LINES` lines at most;
/// from shortly before the first match when `around_match`.
fn said(text: &Text, message: &Message, around_match: bool) -> Vec<Line<'static>> {
    let label = text.label(message.role);
    let label_cells = label.width();
    let mut said = one_line(&message.text);
    let first_match = text.query.highlights(&said).first().map(|hit| hit.start);
    if let Some(start) = first_match.filter(|_| around_match) {
        // About half a line before the match, so it shows in a long message.
        let before = &said[..start];
        let room = text.cells.saturating_sub(label_cells) / 2;
        if before.width() > room {
            said = format!("{}{}", fit_left(before, room), &said[start..]);
        }
    }
    text.paragraph(&said, &label, MESSAGE_LINES, Style::new())
        .lines
}
