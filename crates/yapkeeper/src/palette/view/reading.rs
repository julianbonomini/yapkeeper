//! The selected session's whole conversation, drawn in place of the query,
//! the results and the preview: its title and where it ran, then every
//! message wrapped by terminal cells with what the query's words match
//! highlighted, and each tool call on one line.

use std::path::Path;
use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Drawn, Text, Theme, location};
use crate::line::{age, availability_label, fit, one_line};
use crate::palette::Palette;
use crate::{Listed, Query, Role};

/// Draws the title and where the session ran into `heading`, and the part
/// of its conversation the reading position shows into `body`.
pub(super) fn render(
    palette: &mut Palette,
    theme: &Theme,
    frame: &mut Frame,
    heading: Rect,
    body: Rect,
) {
    let Some(listed) = palette.selected() else {
        return;
    };
    let session = &listed.session;
    let text = Text {
        query: &palette.parsed,
        theme,
        cells: usize::from(body.width.saturating_sub(2)),
        provider: session.provider,
    };
    // Tool calls are searched only on request, so only then highlighted.
    let unsearched = Query::default();
    let tools = Text {
        query: match palette.parsed.tools() {
            true => &palette.parsed,
            false => &unsearched,
        },
        ..text
    };
    let mut lines: Vec<Line> = Vec::new();
    let mut hits: Vec<usize> = Vec::new();
    for (n, message) in palette.conversation.iter().enumerate() {
        if n > 0 {
            lines.push(Line::raw(""));
        }
        let label = text.label(message.role);
        let drawn = match message.role {
            Role::ToolCall | Role::ToolOutput => {
                tools.paragraph(&message.text, &label, 1, theme.dim)
            }
            _ => paragraphs(&text, &message.text, &label),
        };
        hits.extend(drawn.hits.iter().map(|hit| lines.len() + hit));
        lines.extend(drawn.lines);
    }

    let height = usize::from(body.height);
    let Some(reading) = &mut palette.reading else {
        return;
    };
    let settled = reading.settle(lines.len(), height, &hits);
    let shown: Vec<Line> = lines.into_iter().skip(settled.top).take(height).collect();
    frame.render_widget(
        // A cell of margin on either side.
        Paragraph::new(shown).block(Block::new().padding(Padding::horizontal(1))),
        body,
    );

    let found = match settled.current {
        Some(current) => format!("match {} / {}", current + 1, hits.len()),
        None => String::new(),
    };
    let home = palette.home.as_deref();
    let lines = heading_lines(listed, home, &found, theme, usize::from(heading.width));
    frame.render_widget(Paragraph::new(lines), heading);
}

/// The session's title with `found` on the right, then its provider, age,
/// where it ran and whether it can resume, `width` cells wide.
fn heading_lines(
    listed: &Listed,
    home: Option<&Path>,
    found: &str,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let session = &listed.session;
    // A space before the title and one before what was found.
    let title = fit(
        &one_line(&session.title),
        width.saturating_sub(2 + found.width()),
    );
    let gap = width.saturating_sub(1 + title.width() + found.width());
    let label = availability_label(listed.location.availability);
    let label = match label.is_empty() {
        true => String::new(),
        false => format!("  {label}"),
    };
    let ran = format!(
        " {} · {} ago · ",
        session.provider,
        age(session.last_active, SystemTime::now())
    );
    let place_cells = width.saturating_sub(ran.width() + 2 + label.width());
    let (icon, place) = location(listed, home, place_cells);
    vec![
        Line::from(vec![
            Span::styled(format!(" {title}"), theme.title),
            Span::raw(" ".repeat(gap)),
            Span::styled(found.to_string(), theme.dim),
        ]),
        Line::from(vec![
            Span::styled(format!("{ran}{icon} {place}"), theme.dim),
            Span::styled(label, theme.label),
        ]),
    ]
}

/// Every line of `said`, wrapped whole, after `label`; a run of blank lines
/// keeps one.
fn paragraphs(text: &Text, said: &str, label: &str) -> Drawn {
    let mut drawn = Drawn {
        lines: Vec::new(),
        hits: Vec::new(),
    };
    let mut lead = label.to_string();
    let mut gap = false;
    for line in said.lines() {
        if one_line(line).is_empty() {
            gap = !drawn.lines.is_empty();
            continue;
        }
        if gap {
            drawn.lines.push(Line::raw(""));
            gap = false;
        }
        let part = text.paragraph(line, &lead, usize::MAX, Style::new());
        let at = drawn.lines.len();
        drawn.hits.extend(part.hits.iter().map(|hit| at + hit));
        drawn.lines.extend(part.lines);
        lead = " ".repeat(label.width());
    }
    drawn
}
