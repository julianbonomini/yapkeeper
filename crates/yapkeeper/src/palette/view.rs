//! How the palette looks: its theme, its breakpoints and every line it
//! draws, from the query line, its filters in the accent, down to the
//! completions or the results and the footer, plus the too-small state.
//! The preview's lines are its own module's.

mod preview;

use std::path::Path;
use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Colour, Palette};
use crate::line::{age, availability_label, fit, fit_left, one_line, place};
use crate::{Availability, Listed, Place, TokenKind};

/// The smallest terminal the palette draws in; below it only says so.
const MIN_WIDTH: u16 = 48;
const MIN_HEIGHT: u16 = 12;

/// From this many terminal columns, results keep the left side and the
/// preview the right.
const SPLIT_WIDTH: u16 = 80;

const PROMPT: &str = " › ";
const PLACEHOLDER: &str = "search everything · @where #provider is:state";

/// Wide enough for every age up to `99w`, so ages line up.
const AGE_CELLS: usize = 3;

// Nerd Font icons for where a session ran.
const REPO_ICON: &str = "\u{f401}";
const WORKTREE_ICON: &str = "\u{e725}";
const FOLDER_ICON: &str = "\u{f07b}";
const MISSING_ICON: &str = "\u{f127}";

/// What each part of the palette means, as a style.
struct Theme {
    accent: Style,
    selection: Style,
    dim: Style,
    title: Style,
    /// Text a search word matched; underlined, so it shows without colour.
    highlight: Style,
    /// `orphaned`, `unavailable` and search errors.
    label: Style,
    key: Style,
}

impl Theme {
    fn of(colour: Colour) -> Theme {
        let dim = Style::new().add_modifier(Modifier::DIM);
        let bold = Style::new().add_modifier(Modifier::BOLD);
        match colour {
            Colour::Sage => Theme {
                accent: bold.fg(Color::Indexed(108)),
                selection: bold.bg(Color::Indexed(237)),
                dim,
                title: bold,
                highlight: bold
                    .fg(Color::Indexed(108))
                    .add_modifier(Modifier::UNDERLINED),
                label: Style::new().fg(Color::Yellow),
                key: bold,
            },
            // Meaning survives without colour: the selection is reversed
            // and labels are words.
            Colour::Mono => Theme {
                accent: bold,
                selection: Style::new().add_modifier(Modifier::REVERSED),
                dim,
                title: bold,
                highlight: bold.add_modifier(Modifier::UNDERLINED),
                label: bold,
                key: bold,
            },
        }
    }
}

pub(super) fn render(palette: &mut Palette, frame: &mut Frame) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small(frame, area);
        return;
    }
    let theme = Theme::of(palette.colour);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [query, rule, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);
    let results = match area.width >= SPLIT_WIDTH && palette.completions.is_empty() {
        true => {
            let [results, preview] =
                Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
                    .areas(body);
            preview::render(palette, &theme, frame, preview);
            results
        }
        false => body,
    };
    query_line(palette, &theme, frame, query);
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(rule.width))).style(theme.dim),
        rule,
    );
    match palette.completions.is_empty() {
        true => rows(palette, &theme, frame, results),
        false => completions(palette, &theme, frame, results),
    }
    footer_line(palette, &theme, frame, footer);
}

/// Says the terminal is too small, and how big it must be, centred.
fn too_small(frame: &mut Frame, area: Rect) {
    let [_, message, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    let lines = vec![
        Line::from("terminal too small"),
        Line::from(format!("need at least {MIN_WIDTH}×{MIN_HEIGHT}")),
    ];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), message);
}

/// The prompt, the query with the cursor after it, and the match count on
/// the right.
fn query_line(palette: &Palette, theme: &Theme, frame: &mut Frame, area: Rect) {
    let count = format!("{} / {}", palette.found.len(), palette.catalog.indexed());
    let width = usize::from(area.width);
    // One space keeps the query off the count.
    let room = width.saturating_sub(PROMPT.width() + count.width() + 1);
    // The cursor sits after what was typed, or at the placeholder's start.
    let (typed, typed_cells) = match palette.query.is_empty() {
        true => (vec![Span::styled(fit(PLACEHOLDER, room), theme.dim)], 0),
        false => typed_query(palette, theme, room),
    };
    let cursor_x = area.x + (PROMPT.width() + typed_cells) as u16;
    let typed_width: usize = typed.iter().map(Span::width).sum();
    let gap = width.saturating_sub(PROMPT.width() + typed_width + count.width());
    let mut spans = vec![Span::styled(PROMPT, theme.accent)];
    spans.extend(typed);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(count, theme.dim));
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
    frame.set_cursor_position(Position::new(cursor_x, area.y));
}

/// The query as typed, in at most `cells` cells, keeping its end and
/// starting with `…` when cut, and how many cells it takes. Each filter
/// token is in the accent, as the query parser read it; words and the spaces
/// between tokens stay plain, every space kept so the cursor moves with it.
fn typed_query<'a>(palette: &Palette, theme: &Theme, cells: usize) -> (Vec<Span<'a>>, usize) {
    let query = &palette.query;
    let style_at = |at: usize| {
        let token = palette
            .parsed
            .tokens()
            .iter()
            .find(|token| token.span.contains(&at));
        match token {
            Some(token) if token.kind != TokenKind::Word => theme.accent,
            _ => Style::new(),
        }
    };
    let chars: Vec<(char, Style)> = query
        .char_indices()
        .filter(|(_, c)| !c.is_control())
        .map(|(at, c)| (c, style_at(at)))
        .collect();
    // Cut as `fit_left` cuts the plain text, keeping as many characters
    // from the end.
    let plain: String = chars.iter().map(|&(c, _)| c).collect();
    let fitted = fit_left(&plain, cells);
    // In no cells at all there is no room for the `…` either.
    let ellipsis = fitted != plain && cells > 0;
    let kept = &chars[chars.len() - (fitted.chars().count() - usize::from(ellipsis))..];
    let mut spans: Vec<Span> = Vec::new();
    for &(c, style) in kept {
        match spans.last_mut() {
            Some(span) if span.style == style => span.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), style)),
        }
    }
    if ellipsis {
        spans.insert(0, Span::raw("…"));
    }
    let used = spans.iter().map(Span::width).sum();
    (spans, used)
}

/// One line per way to finish the filter being typed: the token as it
/// would read, in the accent, and how many sessions it lists.
fn completions(palette: &mut Palette, theme: &Theme, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let items: Vec<ListItem> = palette
        .completions
        .iter()
        .map(|completion| {
            let count = completion.count.to_string();
            // A space before the token and at least one before the count,
            // which ends in the last cell, as ages do.
            let token = fit(&completion.token, width.saturating_sub(2 + count.width()));
            let gap = width.saturating_sub(1 + token.width() + count.width());
            ListItem::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(token, theme.accent),
                Span::raw(" ".repeat(gap)),
                Span::styled(count, theme.dim),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    frame.render_stateful_widget(list, area, &mut palette.completion);
}

/// One line per session, or what to try when there are none.
fn rows(palette: &mut Palette, theme: &Theme, frame: &mut Frame, area: Rect) {
    if palette.catalog.indexed() == 0 {
        frame.render_widget(Paragraph::new(" No sessions found."), area);
        return;
    }
    if palette.found.is_empty() {
        let mut lines = vec![Line::from(" No sessions match.")];
        if palette.left_out.temp {
            lines.push(Line::styled(" is:temp shows temp sessions", theme.dim));
        }
        if palette.left_out.hidden {
            lines.push(Line::styled(" is:hidden shows hidden ones", theme.dim));
        }
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let now = SystemTime::now();
    let width = usize::from(area.width);
    let home = palette.home.as_deref();
    let items: Vec<ListItem> = palette
        .found
        .iter()
        .map(|listed| ListItem::new(row(listed, home, theme, width, now)))
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    frame.render_stateful_widget(list, area, &mut palette.selection);
}

/// ` title label  icon location  age`, exactly `width` cells wide when it
/// fits: the location keeps its column and the title gives way.
fn row<'a>(
    listed: &Listed,
    home: Option<&Path>,
    theme: &Theme,
    width: usize,
    now: SystemTime,
) -> Line<'a> {
    let location_cells = (width / 3).clamp(10, 24);
    let label = availability_label(listed.location.availability);
    let label = match label.is_empty() {
        true => String::new(),
        false => format!(" {label}"),
    };
    // A leading space, then a space before the location and before the age.
    let title_cells = width.saturating_sub(1 + label.width() + 1 + location_cells + 1 + AGE_CELLS);
    let title = fit(&one_line(&listed.session.title), title_cells);
    let title_gap = title_cells.saturating_sub(title.width());
    let (icon, place_text) = location(listed, home, location_cells.saturating_sub(2));
    let location_gap = location_cells.saturating_sub(2 + place_text.width());
    let age = age(listed.session.last_active, now);
    Line::from(vec![
        Span::raw(" "),
        Span::raw(title),
        Span::styled(label, theme.label),
        Span::raw(" ".repeat(title_gap + 1)),
        Span::styled(format!("{icon} {place_text}"), theme.dim),
        Span::raw(" ".repeat(location_gap + 1)),
        Span::styled(format!("{age:>AGE_CELLS$}"), theme.dim),
    ])
}

/// The icon for where `listed` ran, and its place fitted to `cells`: a path
/// keeps its end, a project name its start.
fn location(listed: &Listed, home: Option<&Path>, cells: usize) -> (&'static str, String) {
    let text = place(listed, home);
    let text = match listed.location.place {
        Place::Folder => fit_left(&text, cells),
        _ => fit(&text, cells),
    };
    (icon(listed), text)
}

/// The icon for the kind of place `listed` ran in, or for a missing folder.
fn icon(listed: &Listed) -> &'static str {
    match (&listed.location.place, listed.location.availability) {
        (_, Availability::Orphaned) => MISSING_ICON,
        (Place::MainCheckout { .. }, _) => REPO_ICON,
        (Place::Worktree { .. }, _) => WORKTREE_ICON,
        (Place::Folder, _) => FOLDER_ICON,
    }
}

/// Key hints, dropped whole from the right when they don't fit, or the
/// last search error in their place.
fn footer_line(palette: &Palette, theme: &Theme, frame: &mut Frame, area: Rect) {
    if let Some(error) = &palette.error {
        let text = fit(&one_line(error), usize::from(area.width).saturating_sub(1));
        frame.render_widget(Paragraph::new(format!(" {text}")).style(theme.label), area);
        return;
    }
    let esc = match palette.query.is_empty() {
        true => "quit",
        false => "clear",
    };
    let hints = match palette.completions.is_empty() {
        true => [
            Some(("↑↓", "move")),
            Some(("Esc", esc)),
            Some(("^U", "clear")),
            palette.launch_token.as_ref().map(|_| ("^L", "here")),
            Some(("^C", "quit")),
        ],
        false => [
            Some(("↑↓", "choose")),
            Some(("Tab", "insert")),
            Some(("Esc", esc)),
            palette.launch_token.as_ref().map(|_| ("^L", "here")),
            Some(("^C", "quit")),
        ],
    };
    let mut spans = Vec::new();
    let mut used = 0;
    for (key, label) in hints.into_iter().flatten() {
        // A space before each hint and one more between hints.
        let cells = 1 + key.width() + 1 + label.width() + usize::from(used > 0);
        if used + cells > usize::from(area.width) {
            break;
        }
        if used > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(key, theme.key));
        spans.push(Span::styled(format!(" {label}"), theme.dim));
        used += cells;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
