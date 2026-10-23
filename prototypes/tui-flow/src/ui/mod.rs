//! Rendering. The palette lives in `d`; overlays and the conversation view live here.

mod d;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, View};
use crate::data::{Availability, Role, Session};
use crate::text;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Smallest size where the palette still works.
pub const MIN: (u16, u16) = (48, 12);

/// Semantic style tokens. `NO_COLOR` keeps meaning through words and modifiers.
pub struct Theme {
    pub accent: Style,
    pub dim: Style,
    pub warn: Style,
    pub hl: Style,
    pub sel: Style,
    pub key: Style,
    pub bar: Style,
    pub title: Style,
}

/// Prototype knobs, flipped from the settings pane.
pub static THEME: AtomicUsize = AtomicUsize::new(0);
pub static ICONS: AtomicUsize = AtomicUsize::new(0);
pub const THEMES: &[&str] = &[
    "Cyan, reversed selection",
    "Amber, soft selection",
    "Violet, soft selection",
    "Sky, soft selection",
    "Sage, soft selection",
    "Mono",
];
pub const ICON_SETS: &[&str] = &["Unicode", "Nerd Font", "Text tags", "None"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocKind {
    Repo,
    Worktree,
    Folder,
    Missing,
}

/// A location's icon in the chosen set, padded to a fixed width (empty for "None").
pub fn icon(k: LocKind) -> &'static str {
    let set: [&str; 4] = match ICONS.load(Ordering::Relaxed) {
        0 => ["▣ ", "⎇ ", "▢ ", "⊘ "],
        1 => ["\u{e702} ", "\u{e725} ", "\u{f07b} ", "\u{f05e} "],
        2 => ["git  ", "wt   ", "dir  ", "gone "],
        _ => ["", "", "", ""],
    };
    set[k as usize]
}

pub fn session_kind(s: &Session) -> LocKind {
    match (&s.project, &s.branch, s.availability) {
        (_, _, Availability::Orphaned) => LocKind::Missing,
        (Some(_), Some(_), _) => LocKind::Worktree,
        (Some(_), None, _) => LocKind::Repo,
        _ => LocKind::Folder,
    }
}

pub fn theme() -> Theme {
    let mono = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let preset = THEME.load(Ordering::Relaxed);
    let mono = mono || preset == 5;
    let c = |color: Color| {
        if mono {
            Style::new()
        } else {
            Style::new().fg(color)
        }
    };
    let (accent, hl) = match preset {
        1 => (Color::Indexed(214), Color::Indexed(214)),
        2 => (Color::Indexed(141), Color::Indexed(222)),
        3 => (Color::Indexed(75), Color::Indexed(222)),
        4 => (Color::Indexed(108), Color::Indexed(222)),
        _ => (Color::Cyan, Color::Yellow),
    };
    let sel = if preset == 0 || mono {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
            .bg(Color::Indexed(237))
            .add_modifier(Modifier::BOLD)
    };
    Theme {
        accent: c(accent).add_modifier(Modifier::BOLD),
        dim: Style::new().add_modifier(Modifier::DIM),
        warn: c(Color::Yellow),
        hl: c(hl).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        sel,
        key: Style::new().add_modifier(Modifier::BOLD),
        bar: if mono {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new().fg(Color::Black).bg(Color::Magenta)
        },
        title: Style::new().add_modifier(Modifier::BOLD),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = theme();
    let area = f.area();
    if area.width < MIN.0 || area.height < MIN.1 {
        let msg = format!(
            "terminal too small: {}×{}\nneed at least {}×{}",
            area.width, area.height, MIN.0, MIN.1
        );
        f.render_widget(Paragraph::new(msg).centered(), area);
        return;
    }
    let [body, footer, bar] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    match app.view {
        View::Conversation { scroll, cur } => conversation(f, app, body, scroll, cur, &t),
        View::List => d::draw(f, app, body, &t),
    }
    footer_line(f, app, footer, &t);
    proto_bar(f, app, bar, &t);
    if app.sheet.is_some() {
        sheet(f, app, area, &t);
    }
    if app.help {
        help(f, app, area, &t);
    }
    if app.settings_open {
        settings(f, app, area, &t);
    }
}

/// Prototype settings: sensible defaults made visible, plus the visual experiments.
fn settings(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let on = |b: bool| if b { "on " } else { "off" };
    let rows: Vec<(String, String)> = vec![
        (
            "Hide sessions in temp folders".into(),
            on(app.hide_temp).into(),
        ),
        (
            "Hide missing folders in the location list".into(),
            on(app.hide_missing).into(),
        ),
        (
            "List orphaned sessions after the rest".into(),
            on(app.orphans_last).into(),
        ),
        (
            "Accent".into(),
            THEMES[THEME.load(Ordering::Relaxed)].into(),
        ),
        (
            "Location icons".into(),
            ICON_SETS[ICONS.load(Ordering::Relaxed)].into(),
        ),
    ];
    let w = 72.min(area.width.saturating_sub(4));
    let inner = w.saturating_sub(4) as usize;
    let mut lines = vec![Line::raw("")];
    for (n, (label, value)) in rows.iter().enumerate() {
        let lw = inner.saturating_sub(26);
        let st = if n == app.settings_sel {
            t.sel
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled(text::pad(label, lw), st),
            Span::styled(text::pad_left(value, 26), st.patch(t.accent)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Reads", t.dim));
    for root in [
        "~/.claude/projects",
        "~/.codex/sessions",
        "~/.codex/archived_sessions",
    ] {
        lines.push(Line::styled(format!("  {root}"), t.dim));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Typing is:temp shows temp sessions anyway.",
        t.dim,
    ));
    lines.push(keys_line(
        &[("↑↓", "move"), ("Enter/←→", "change"), ("Esc", "close")],
        t,
    ));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .title(" Settings ")
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        r,
    );
}

/// Key hints for the current context. The footer and the help screen both read this.
pub fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.sheet.is_some() {
        return vec![("Enter", "resume"), ("y", "copy ID"), ("Esc", "cancel")];
    }
    if let View::Conversation { .. } = app.view {
        return vec![
            ("Enter", "resume"),
            ("j/k", "scroll"),
            ("n/N", "next match"),
            ("Esc", "back"),
            ("?", "help"),
        ];
    }
    if !app.suggestions().is_empty() {
        return vec![
            ("Enter", "pick"),
            ("↑↓", "move"),
            ("type", "narrow"),
            ("Esc", "clear"),
        ];
    }
    vec![
        ("↑↓", "move"),
        ("Enter", "resume"),
        ("→", "read"),
        ("Tab", "locations"),
        ("^L", "this folder"),
        ("#", "agent"),
        ("is:", "orphaned/hidden"),
        ("^X", "hide"),
        ("^T", "tools"),
        ("Esc", "quit"),
    ]
}

fn footer_line(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    if let Some((msg, _)) = &app.toast {
        let line = Line::from(Span::styled(format!(" {msg}"), t.accent));
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (k, label) in keys(app) {
        let w = text::width(k) + text::width(label) + 3;
        if used + w > area.width as usize {
            break;
        }
        spans.push(Span::styled(k, t.key));
        spans.push(Span::styled(format!(" {label}  "), t.dim));
        used += w;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Prototype-only chrome: variant switcher plus the full relevant state.
fn proto_bar(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let sel = app
        .selected()
        .map(|i| app.sessions[i].id.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "-".into());
    let on = |b: bool| if b { "on" } else { "off" };
    let s = format!(
        " PROTOTYPE · {} │ {}×{} q=“{}” sel={} tools={} │ {}",
        app.source,
        app.size.0,
        app.size.1,
        app.query,
        sel,
        on(app.include_tools),
        app.last
    );
    let line = text::pad(&s, area.width as usize);
    f.render_widget(Paragraph::new(line).style(t.bar), area);
}

/// Words, not glyphs, so the state survives `NO_COLOR`. Normal rows carry nothing.
pub fn status(s: &Session, t: &Theme) -> Option<Span<'static>> {
    let word = match (s.availability, s.hidden) {
        (Availability::Orphaned, true) => "orphaned, hidden",
        (Availability::Unknown, true) => "unavailable, hidden",
        (Availability::Orphaned, false) => "orphaned",
        (Availability::Unknown, false) => "unavailable",
        (Availability::Resumable, true) => "hidden",
        (Availability::Resumable, false) => return None,
    };
    let style = if s.availability == Availability::Resumable {
        t.dim
    } else {
        t.warn
    };
    Some(Span::styled(word, style))
}

fn role_label(s: &Session, r: Role) -> &'static str {
    match r {
        Role::User => "you",
        Role::Assistant => s.provider.name(),
        Role::Tool => "tool",
    }
}

pub fn rule(w: u16, t: &Theme) -> Line<'static> {
    Line::styled("─".repeat(w as usize), t.dim)
}

/// Preview for the side pane: identity, location, then the passages that aid recognition.
pub fn preview_lines(app: &App, i: usize, w: usize, t: &Theme) -> Vec<Line<'static>> {
    let s = &app.sessions[i];
    let terms = app.terms();
    let mut out = Vec::new();
    for l in text::wrap(&s.display_title(), w) {
        out.push(Line::styled(l, t.title));
    }
    let mut meta = format!(
        "{} · {} ago · {} messages",
        s.provider.name(),
        text::ago(s.minutes_ago),
        s.msgs.len()
    );
    if !s.subagents.is_empty() {
        meta.push_str(&format!(" · {} subagent", s.subagents.len()));
    }
    if !s.prs.is_empty() {
        let prs: Vec<String> = s.prs.iter().map(|n| format!("#{n}")).collect();
        meta.push_str(&format!(" · PR {}", prs.join(", ")));
    }
    out.push(Line::styled(text::trunc(&meta, w), t.dim));
    out.push(Line::styled(
        text::trunc_left(&format!("{}{}", icon(session_kind(s)), s.dir()), w),
        t.dim,
    ));
    if let Some(b) = &s.branch {
        out.push(Line::styled(text::trunc(&format!("branch {b}"), w), t.dim));
    }
    match s.availability {
        Availability::Orphaned => {
            out.push(Line::styled("orphaned: directory no longer exists", t.warn))
        }
        Availability::Unknown => {
            out.push(Line::styled("unavailable: can't inspect directory", t.warn))
        }
        Availability::Resumable => {}
    }
    if s.hidden {
        out.push(Line::styled("hidden", t.dim));
    }
    out.push(Line::raw(""));

    let passage = |out: &mut Vec<Line<'static>>, role: Role, body: &str, max_lines: usize| {
        let label = role_label(s, role);
        let pad = 8;
        let style = if role == Role::Tool {
            t.dim
        } else {
            Style::new()
        };
        for (n, l) in text::wrap(body, w.saturating_sub(pad))
            .into_iter()
            .take(max_lines)
            .enumerate()
        {
            let lead = if n == 0 {
                text::pad(label, pad)
            } else {
                " ".repeat(pad)
            };
            let mut spans = vec![Span::styled(lead, t.dim)];
            spans.extend(text::highlight(&l, &terms, style, t.hl));
            out.push(Line::from(spans));
        }
    };

    if let Some(r) = &s.recap {
        out.push(Line::styled("Recap", t.accent));
        for l in text::wrap(r, w).into_iter().take(5) {
            out.push(Line::raw(l));
        }
        out.push(Line::raw(""));
    }
    let info = app.matches[i].clone().unwrap_or_default();
    if !terms.is_empty() {
        let n = info.msgs.len();
        out.push(Line::styled(
            format!("Matches ({n}{})", if info.title { ", title" } else { "" }),
            t.accent,
        ));
        for &m in info.msgs.iter().take(3) {
            let msg = &s.msgs[m];
            passage(
                &mut out,
                msg.role,
                &text::snippet(&msg.text, &terms, 400),
                3,
            );
        }
        out.push(Line::raw(""));
    }
    if let Some(first) = s.msgs.iter().find(|m| m.role == Role::User) {
        out.push(Line::styled("Opening", t.accent));
        passage(&mut out, Role::User, &first.text, 3);
        out.push(Line::raw(""));
    }
    let last_user = s.msgs.iter().rposition(|m| m.role == Role::User);
    if let Some(lu) = last_user.filter(|&lu| lu > 0) {
        out.push(Line::styled("Latest", t.accent));
        for m in s.msgs[lu..].iter().filter(|m| m.role != Role::Tool) {
            passage(&mut out, m.role, &m.text, 4);
        }
    }
    out
}

/// One-line passage: the match while searching, else the latest prompt that isn't the title.
pub fn passage(app: &App, i: usize, w: usize) -> String {
    let s = &app.sessions[i];
    let terms = app.terms();
    let info = app.matches[i].clone().unwrap_or_default();
    if let Some(&m) = info.msgs.first().filter(|_| !terms.is_empty()) {
        return text::snippet(&s.msgs[m].text, &terms, w);
    }
    let title = s.display_title();
    let pick = s
        .msgs
        .iter()
        .rev()
        .find(|m| m.role == Role::User && m.text.lines().next() != Some(title.as_str()))
        .or_else(|| s.msgs.iter().rev().find(|m| m.role == Role::Assistant));
    text::trunc(
        &pick.map(|m| m.text.replace('\n', " ")).unwrap_or_default(),
        w,
    )
}

/// Full conversation lines and the indices of lines that hold a match.
pub fn conversation_lines(app: &App, w: usize) -> (Vec<Line<'static>>, Vec<usize>) {
    let t = theme();
    let Some(i) = app.selected() else {
        return (vec![], vec![]);
    };
    let s = &app.sessions[i];
    let terms = app.terms();
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    let pad = 8;
    let mut push_msg = |lines: &mut Vec<Line<'static>>, role: Role, body: &str| {
        let style = if role == Role::Tool {
            t.dim
        } else {
            Style::new()
        };
        let searchable = role != Role::Tool || app.include_tools;
        for (n, l) in text::wrap(body, w.saturating_sub(pad))
            .into_iter()
            .enumerate()
        {
            let lead = if n == 0 {
                text::pad(role_label(s, role), pad)
            } else {
                " ".repeat(pad)
            };
            if searchable && terms.iter().any(|q| text::contains(&l, q)) {
                hits.push(lines.len());
            }
            let mut spans = vec![Span::styled(lead, t.dim)];
            if searchable {
                spans.extend(text::highlight(&l, &terms, style, t.hl));
            } else {
                spans.push(Span::styled(l, style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::raw(""));
    };
    for m in &s.msgs {
        push_msg(&mut lines, m.role, &m.text);
    }
    for (n, sub) in s.subagents.iter().enumerate() {
        // Real histories only count subagent files; there's nothing to show.
        if sub.is_empty() {
            continue;
        }
        lines.push(Line::styled(
            format!("── subagent transcript {} ──", n + 1),
            t.dim,
        ));
        for m in sub {
            push_msg(&mut lines, m.role, &m.text);
        }
    }
    (lines, hits)
}

fn conversation(f: &mut Frame, app: &App, area: Rect, scroll: usize, cur: usize, t: &Theme) {
    let Some(i) = app.selected() else { return };
    let s = &app.sessions[i];
    let [head, sep, body] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    let w = area.width as usize;
    let (lines, hits) = conversation_lines(app, w.saturating_sub(4));
    let right = if hits.is_empty() {
        String::new()
    } else {
        format!("match {}/{} ", cur.min(hits.len() - 1) + 1, hits.len())
    };
    let st = status(s, t);
    let st_w = st.as_ref().map_or(0, |s| s.width() + 2);
    let lead = format!(
        " {} · {} ago · ",
        s.provider.name(),
        text::ago(s.minutes_ago)
    );
    let loc_w = w.saturating_sub(lead.len() + st_w + right.len() + 1);
    let mut meta = vec![
        Span::styled(lead, t.dim),
        Span::styled(text::pad(&s.location_fit(loc_w), loc_w), t.dim),
    ];
    if let Some(st) = st {
        meta.push(Span::raw("  "));
        meta.push(st);
    }
    meta.push(Span::raw(" "));
    meta.push(Span::styled(right, t.accent));
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(" {}", text::trunc(&s.display_title(), w - 1)),
                t.title,
            ),
            Line::from(meta),
        ]),
        head,
    );
    f.render_widget(Paragraph::new(rule(area.width, t)), sep);
    let total = lines.len();
    let view: Vec<Line> = lines.into_iter().skip(scroll).collect();
    f.render_widget(
        Paragraph::new(Text::from(view)).block(Block::new().borders(Borders::NONE)),
        body.inner(ratatui::layout::Margin::new(2, 0)),
    );
    if total > body.height as usize {
        let pct = (scroll + body.height as usize).min(total) * 100 / total;
        let tag = format!(" {pct}% ");
        let r = Rect::new(
            body.right().saturating_sub(tag.len() as u16 + 1),
            body.bottom().saturating_sub(1),
            tag.len() as u16,
            1,
        );
        f.render_widget(Paragraph::new(Span::styled(tag, t.dim)), r);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn sheet(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Some(sh) = &app.sheet else { return };
    let s = &app.sessions[sh.session];
    let w = 70.min(area.width.saturating_sub(4));
    let inner_w = w.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = vec![];
    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(text::pad(k, 10), t.dim),
            Span::raw(text::trunc_left(&v, inner_w.saturating_sub(10))),
        ])
    };
    let argv = s.resume_argv(&s.dir()).join(" ");
    match s.availability {
        Availability::Resumable => {
            lines.push(Line::styled(
                text::trunc(&s.display_title(), inner_w),
                t.title,
            ));
            lines.push(Line::raw(""));
            lines.push(kv("directory", s.dir()));
            for (n, l) in text::wrap(&argv, inner_w.saturating_sub(10))
                .into_iter()
                .enumerate()
            {
                lines.push(kv(if n == 0 { "command" } else { "" }, l));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Yapkeeper exits and starts the agent here.",
                t.dim,
            ));
            lines.push(keys_line(
                &[("Enter", "resume"), ("y", "copy ID"), ("Esc", "cancel")],
                t,
            ));
        }
        Availability::Orphaned | Availability::Unknown => {
            let orphan = s.availability == Availability::Orphaned;
            lines.push(Line::styled(
                if orphan {
                    "Directory missing"
                } else {
                    "Can't check directory"
                },
                t.warn.add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));
            lines.push(kv("directory", s.dir()));
            lines.push(kv("session", s.id.clone()));
            let why = if orphan {
                "This directory no longer exists. Yapkeeper will not resume the session somewhere else unless you choose to."
            } else {
                "The directory could not be inspected (volume unmounted or no permission). It may still exist."
            };
            lines.push(Line::raw(""));
            for l in text::wrap(why, inner_w) {
                lines.push(Line::raw(l));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("c ", t.key),
                Span::raw("resume in the launch directory instead:"),
            ]));
            lines.push(Line::styled(
                format!("  {}", text::trunc_left(&app.cwd, inner_w - 2)),
                t.dim,
            ));
            if s.provider == crate::data::Provider::Claude {
                lines.push(Line::styled(
                    text::trunc(
                        "  Claude looks up sessions by project directory; may not find it.",
                        inner_w,
                    ),
                    t.dim,
                ));
            }
            let mut ks = vec![("y", "copy ID"), ("Esc", "cancel")];
            if !orphan {
                ks.insert(0, ("r", "check again"));
            }
            lines.push(keys_line(&ks, t));
        }
    }
    if let Some(note) = &sh.note {
        lines.push(Line::styled(text::trunc(note, inner_w), t.warn));
    }
    let h = lines.len() as u16 + 2;
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Resume ")
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        r,
    );
}

fn keys_line(ks: &[(&'static str, &'static str)], t: &Theme) -> Line<'static> {
    let mut spans = vec![];
    for (k, l) in ks {
        spans.push(Span::styled(*k, t.key));
        spans.push(Span::styled(format!(" {l}   "), t.dim));
    }
    Line::from(spans)
}

fn help(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let mut lines = vec![
        Line::styled("Yapkeeper".to_string(), t.title),
        Line::raw(""),
    ];
    for (k, l) in keys(app) {
        lines.push(Line::from(vec![
            Span::styled(text::pad(k, 8), t.key),
            Span::raw(l),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Ctrl+C quit", t.dim));
    lines.push(Line::styled("any key closes", t.dim));
    let r = centered(area, 44, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Keys ")
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        r,
    );
}
