//! Variant D: a floating command palette. One input searches everywhere; `@` narrows to a
//! location with inline completion, `#` to an agent, `is:` to a state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Padding, Paragraph};

use super::{Theme, preview_lines, rule, status};
use crate::app::{App, Kind, Sugg};
use crate::text;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    let r = area;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(t.dim);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let [input, sep, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    f.render_widget(
        Paragraph::new(input_line(app, input.width as usize, t)),
        input,
    );
    f.render_widget(Paragraph::new(rule(inner.width, t)), sep);

    let sugg = app.suggestions();
    if !sugg.is_empty() {
        suggestions(f, app, body, &sugg, t);
        return;
    }
    let split = inner.width >= 80;
    let (list_area, preview_area) = if split {
        let [l, p] = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
            .areas(body);
        (l, Some(p))
    } else {
        (body, None)
    };
    results(f, app, list_area, t);
    if let (Some(p), Some(i)) = (preview_area, app.selected()) {
        let block = Block::new()
            .borders(Borders::LEFT)
            .border_style(t.dim)
            .padding(Padding::horizontal(1));
        let w = block.inner(p).width as usize;
        f.render_widget(Paragraph::new(preview_lines(app, i, w, t)).block(block), p);
    }
}

/// Query with filter tokens colored, so `@acme-api` reads as a chip, not text.
fn input_line(app: &App, w: usize, t: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(" › ", t.accent)];
    if app.query.is_empty() {
        spans.push(Span::styled("▏", t.accent));
        spans.push(Span::styled(
            "search everything · Tab locations · #agent · is orphaned · not temp",
            t.dim,
        ));
    } else {
        // Color what the parser treats as filters, so the query shows how it was read.
        let terms = app.terms();
        for (n, tok) in app.query.split(' ').enumerate() {
            if n > 0 {
                spans.push(Span::raw(" "));
            }
            let is_term = terms.iter().any(|x| x == tok);
            let style = if tok.is_empty() || is_term {
                Style::new()
            } else {
                t.accent
            };
            spans.push(Span::styled(tok.to_string(), style));
        }
        spans.push(Span::styled("▏", t.accent));
    }
    let total = app.sessions.iter().filter(|s| !s.hidden).count();
    let mut right = format!("{} / {total}", app.hits.len());
    if app.include_tools {
        right.push_str(" · +tools");
    }
    let temp = app.temp_hidden();
    if temp > 0 {
        right = format!("{temp} in temp hidden · {right}");
    }
    let used: usize = spans.iter().map(|s| s.width()).sum();
    if used + right.len() + 2 < w {
        spans.push(Span::raw(" ".repeat(w - used - right.len() - 1)));
        spans.push(Span::styled(right, t.dim));
    }
    Line::from(spans)
}

fn results(f: &mut Frame, app: &mut App, area: Rect, t: &Theme) {
    if app.hits.is_empty() {
        let msg = if app.query.contains("is:hidden") || app.query.is_empty() {
            "Nothing matches."
        } else {
            "Nothing matches. Hidden sessions need is:hidden; tool output needs ^T."
        };
        f.render_widget(Paragraph::new(Span::styled(format!(" {msg}"), t.dim)), area);
        return;
    }
    let w = area.width as usize;
    let terms = app.terms();
    let items: Vec<ListItem> = app
        .hits
        .iter()
        .map(|&i| {
            let s = &app.sessions[i];
            let st = status(s, t);
            let st_w = st.as_ref().map_or(0, |s| s.width() + 1);
            let loc_w = if w >= 50 { (w / 3).min(24) } else { 0 };
            let title_w = w.saturating_sub(1 + st_w + loc_w + 1 + 5);
            let mut spans = vec![Span::raw(" ")];
            spans.extend(text::highlight(
                &text::pad(&s.display_title(), title_w),
                &terms,
                Style::new(),
                t.hl,
            ));
            if let Some(st) = st {
                spans.push(Span::raw(" "));
                spans.push(st);
            }
            if loc_w > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    {
                        let ic = super::icon(super::session_kind(s));
                        let lw = loc_w.saturating_sub(text::width(ic));
                        text::pad(&format!("{ic}{}", s.location_fit(lw)), loc_w)
                    },
                    t.dim,
                ));
            }
            spans.push(Span::styled(
                text::pad_left(&text::ago(s.minutes_ago), 5),
                t.dim,
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();
    app.list_state.select(Some(app.sel));
    f.render_stateful_widget(
        List::new(items).highlight_style(t.sel),
        area,
        &mut app.list_state,
    );
}

fn suggestions(f: &mut Frame, app: &mut App, area: Rect, sugg: &[Sugg], t: &Theme) {
    let w = area.width as usize;
    let heading = match app.completing().map(|c| c.0) {
        Some(Kind::Loc) => " Locations · projects with their worktrees, then folders",
        Some(Kind::Agent) => " Agents",
        Some(Kind::Neg) => " Exclude",
        _ => " States",
    };
    let mut items = vec![ListItem::new(Line::styled(heading, t.dim))];
    for sg in sugg {
        let count = format!("{:>5} ", sg.count);
        let indent = if sg.depth > 0 { "    " } else { " " };
        let ic = match sg.kind {
            Some(k) => super::icon(k),
            None => "",
        };
        let indent = format!("{indent}{ic}");
        let iw = text::width(&indent);
        let label_w = (w / 3).max(16).min(w.saturating_sub(count.len() + iw));
        let detail_w = w.saturating_sub(iw + label_w + count.len() + 2);
        let style = if sg.depth > 0 { Style::new() } else { t.accent };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(indent.clone(), t.dim),
            Span::styled(
                text::pad(&text::trunc_left(&sg.label, label_w), label_w),
                style,
            ),
            Span::raw("  "),
            Span::styled(
                text::pad(&text::trunc_left(&sg.detail, detail_w), detail_w),
                t.dim,
            ),
            Span::styled(count, t.dim),
        ])));
    }
    app.list_state.select(Some(app.sel + 1));
    f.render_stateful_widget(
        List::new(items).highlight_style(t.sel),
        area,
        &mut app.list_state,
    );
}
