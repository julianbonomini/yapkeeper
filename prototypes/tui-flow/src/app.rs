//! Palette state and key handling.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::data::{Availability, Role, Session};
use crate::text;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    List,
    Conversation { scroll: usize, cur: usize },
}

#[derive(Clone, Default, Debug)]
pub struct MatchInfo {
    pub title: bool,
    /// Indices of matching messages, in order.
    pub msgs: Vec<usize>,
}

pub struct Sheet {
    pub session: usize,
    pub note: Option<String>,
}

pub struct App {
    pub sessions: Vec<Session>,
    pub source: String,
    pub cwd: String,
    pub query: String,
    pub include_tools: bool,
    pub matches: Vec<Option<MatchInfo>>,
    pub hits: Vec<usize>,
    pub sel: usize,
    pub view: View,
    pub sheet: Option<Sheet>,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    pub size: (u16, u16),
    pub last: String,
    pub receipt: Option<String>,
    pub quit: bool,
    pub list_state: ListState,
}

fn under(dir: &str, prefix: &str) -> bool {
    dir == prefix || dir.starts_with(&format!("{prefix}/"))
}

impl App {
    pub fn new(sessions: Vec<Session>, source: String, cwd: &str) -> Self {
        let mut app = App {
            sessions,
            source,
            cwd: cwd.trim_end_matches('/').to_string(),
            query: String::new(),
            include_tools: false,
            matches: vec![],
            hits: vec![],
            sel: 0,
            view: View::List,
            sheet: None,
            help: false,
            toast: None,
            size: (100, 30),
            last: "start".into(),
            receipt: None,
            quit: false,
            list_state: ListState::default(),
        };
        app.recompute();
        app
    }

    /// Free-text terms; `@location`, `#agent`, and `is:state` tokens are filters.
    pub fn terms(&self) -> Vec<String> {
        self.query
            .split_whitespace()
            .filter(|t| !t.starts_with('@') && !t.starts_with('#') && !t.starts_with("is:"))
            .map(str::to_string)
            .collect()
    }

    fn match_session(&self, s: &Session, terms: &[String]) -> Option<MatchInfo> {
        if terms.is_empty() {
            return Some(MatchInfo::default());
        }
        let searchable = |role: Role| role != Role::Tool || self.include_tools;
        let title = s.display_title();
        let mut info = MatchInfo::default();
        for t in terms {
            let in_title = text::contains(&title, t);
            let in_msgs = s
                .msgs
                .iter()
                .any(|m| searchable(m.role) && text::contains(&m.text, t));
            if !(in_title || in_msgs) {
                return None;
            }
            info.title |= in_title;
        }
        info.msgs = s
            .msgs
            .iter()
            .enumerate()
            .filter(|(_, m)| searchable(m.role) && terms.iter().any(|t| text::contains(&m.text, t)))
            .map(|(i, _)| i)
            .collect();
        Some(info)
    }

    /// Filter tokens. Hidden sessions appear only with `is:hidden`.
    fn filters_ok(&self, s: &Session) -> bool {
        let mut want_hidden = false;
        for tok in self.query.split_whitespace() {
            let low = tok.to_ascii_lowercase();
            if let Some(loc) = low.strip_prefix('@') {
                let hay = format!("{} {}", s.location(), s.dir()).to_ascii_lowercase();
                if !hay.contains(loc) {
                    return false;
                }
            } else if let Some(agent) = low.strip_prefix('#') {
                if !s.provider.name().starts_with(agent) {
                    return false;
                }
            } else if let Some(state) = low.strip_prefix("is:") {
                let ok = match state {
                    "hidden" => {
                        want_hidden = true;
                        s.hidden
                    }
                    "orphaned" | "orphan" => s.availability == Availability::Orphaned,
                    "unavailable" => s.availability == Availability::Unknown,
                    "" => true,
                    _ => false,
                };
                if !ok {
                    return false;
                }
            }
        }
        !s.hidden || want_hidden
    }

    pub fn recompute(&mut self) {
        let terms = self.terms();
        let prev = self.selected();
        self.matches = self
            .sessions
            .iter()
            .map(|s| self.match_session(s, &terms))
            .collect();
        let mut hits: Vec<usize> = (0..self.sessions.len())
            .filter(|&i| self.matches[i].is_some() && self.filters_ok(&self.sessions[i]))
            .collect();
        hits.sort_by_key(|&i| {
            let title = self.matches[i].as_ref().is_some_and(|m| m.title);
            (!terms.is_empty() && !title, self.sessions[i].minutes_ago)
        });
        self.hits = hits;
        self.sel = prev
            .and_then(|p| self.hits.iter().position(|&i| i == p))
            .unwrap_or(0);
    }

    pub fn visible_total(&self) -> usize {
        self.sessions.iter().filter(|s| !s.hidden).count()
    }

    pub fn selected(&self) -> Option<usize> {
        self.hits.get(self.sel).copied()
    }

    fn move_sel(&mut self, d: isize) {
        if self.hits.is_empty() {
            return;
        }
        let max = self.hits.len() as isize - 1;
        self.sel = (self.sel as isize + d).clamp(0, max) as usize;
    }

    fn page(&self) -> isize {
        (self.size.1 as isize - 8).max(3)
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn tick(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(3))
        {
            self.toast = None;
        }
    }

    fn toggle_hidden(&mut self) {
        let Some(i) = self.selected() else { return };
        let s = &mut self.sessions[i];
        s.hidden = !s.hidden;
        let title = text::trunc(&s.display_title(), 32);
        let msg = if s.hidden {
            format!("Hidden “{title}” · find it with is:hidden")
        } else {
            format!("Unhid “{title}”")
        };
        self.last = msg.clone();
        self.toast(msg);
        self.recompute();
    }

    fn toggle_tools(&mut self) {
        self.include_tools = !self.include_tools;
        self.recompute();
        let m = if self.include_tools {
            "Searching tool calls too"
        } else {
            "Searching messages only"
        };
        self.toast(m);
        self.last = m.into();
    }

    fn open_conversation(&mut self) {
        if self.selected().is_some() {
            self.view = View::Conversation { scroll: 0, cur: 0 };
            self.jump_match(0);
            self.last = "open conversation".into();
        }
    }

    fn open_sheet(&mut self) {
        if let Some(i) = self.selected() {
            self.sheet = Some(Sheet {
                session: i,
                note: None,
            });
            self.last = "resume sheet".into();
        }
    }

    /// Lines in the conversation view that hold a match, for `n`/`N`.
    pub fn match_lines(&self) -> Vec<usize> {
        let w = self.size.0.saturating_sub(4) as usize;
        crate::ui::conversation_lines(self, w).1
    }

    fn jump_match(&mut self, d: isize) {
        let lines = self.match_lines();
        if let View::Conversation { scroll, cur } = &mut self.view {
            if lines.is_empty() {
                return;
            }
            let n = lines.len() as isize;
            *cur = ((*cur as isize + d).rem_euclid(n)) as usize;
            *scroll = lines[*cur].saturating_sub(2);
        }
    }

    fn finish(&mut self, i: usize, dir: String, explicit: bool) {
        let s = &self.sessions[i];
        let argv = s
            .resume_argv(&dir)
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    format!("'{a}'")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let how = if explicit {
            "  (directory chosen explicitly; not the recorded one)\n"
        } else {
            ""
        };
        self.receipt = Some(format!(
            "yapkeeper prototype: simulated resume, no agent launched.\n  title: {}\n  cwd:   {dir}\n  argv:  {argv}\n{how}",
            s.display_title()
        ));
        self.quit = true;
    }

    /// The `@` token being typed, if the cursor is still inside it.
    pub fn completing(&self) -> Option<String> {
        if self.query.ends_with(' ') {
            return None;
        }
        let last = self.query.split_whitespace().last()?;
        last.strip_prefix('@').map(str::to_ascii_lowercase)
    }

    /// Location completions: (label, sessions, depth). Projects come first with their
    /// worktrees nested beneath; groups are ordered by most recent activity.
    pub fn loc_suggestions(&self) -> Vec<(String, usize, u16)> {
        let Some(part) = self.completing() else {
            return vec![];
        };
        // (group, label, depth, count, recency)
        let mut rows: Vec<(String, String, u16, usize, u32)> = Vec::new();
        for s in self.sessions.iter().filter(|s| !s.hidden) {
            let entries = match (&s.project, &s.branch) {
                (Some(p), Some(b)) => vec![
                    (p.clone(), p.clone(), 0),
                    (p.clone(), format!("{p}:{b}"), 1),
                ],
                (Some(p), None) => vec![(p.clone(), p.clone(), 0)],
                _ => vec![(s.root.clone(), s.root.clone(), 0)],
            };
            for (g, l, d) in entries {
                match rows.iter_mut().find(|r| r.1 == l) {
                    Some(r) => {
                        r.3 += 1;
                        r.4 = r.4.min(s.minutes_ago);
                    }
                    None => rows.push((g, l, d, 1, s.minutes_ago)),
                }
            }
        }
        rows.retain(|r| r.1.to_ascii_lowercase().contains(&part));
        let group_recency = |g: &str| {
            rows.iter()
                .filter(|r| r.0 == g)
                .map(|r| r.4)
                .min()
                .unwrap_or(u32::MAX)
        };
        let mut keyed: Vec<_> = rows
            .iter()
            .map(|r| ((group_recency(&r.0), r.0.clone(), r.2, r.4), r))
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed
            .into_iter()
            .map(|(_, r)| (r.1.clone(), r.3, r.2))
            .collect()
    }

    fn complete(&mut self) {
        let sugg = self.loc_suggestions();
        let Some((label, _, _)) = sugg.get(self.sel) else {
            return;
        };
        let keep: Vec<&str> = self.query.split_whitespace().collect();
        let head = keep[..keep.len() - 1].join(" ");
        self.query = if head.is_empty() {
            format!("@{label} ")
        } else {
            format!("{head} @{label} ")
        };
        self.sel = 0;
        self.recompute();
        self.last = format!("location → {label}");
    }

    fn launch_location(&self) -> Option<String> {
        self.sessions
            .iter()
            .filter(|s| !s.root.is_empty() && under(&self.cwd, &s.root))
            .max_by_key(|s| s.root.len())
            .map(|s| match (&s.project, &s.branch) {
                (Some(p), Some(b)) => format!("{p}:{b}"),
                (Some(p), None) => p.clone(),
                _ => s.root.clone(),
            })
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.sheet.is_some() {
            self.sheet_key(key);
            return;
        }
        if let View::Conversation { .. } = self.view {
            self.conversation_key(key);
            return;
        }
        self.palette_key(key, ctrl);
    }

    fn sheet_key(&mut self, key: KeyEvent) {
        let Some(sheet) = &mut self.sheet else { return };
        let i = sheet.session;
        let s = &self.sessions[i];
        match (key.code, s.availability) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => {
                self.sheet = None;
                self.last = "resume cancelled".into();
            }
            (KeyCode::Enter, Availability::Resumable) => self.finish(i, s.dir(), false),
            (KeyCode::Enter, _) => {
                sheet.note = Some("Not resumable from its recorded directory.".into());
            }
            (KeyCode::Char('c'), Availability::Orphaned | Availability::Unknown) => {
                let cwd = self.cwd.clone();
                self.finish(i, cwd, true);
            }
            (KeyCode::Char('r'), Availability::Unknown) => {
                sheet.note = Some(format!("Checked again: still can't inspect {}.", s.root));
            }
            (KeyCode::Char('y'), _) => {
                self.toast("Copied session ID (simulated)");
                self.last = "copy id".into();
            }
            _ => {}
        }
    }

    fn conversation_key(&mut self, key: KeyEvent) {
        let page = self.page() as usize;
        let total = crate::ui::conversation_lines(self, self.size.0.saturating_sub(4) as usize)
            .0
            .len();
        let max = total.saturating_sub(3);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                self.view = View::List;
                self.last = "back to list".into();
            }
            KeyCode::Enter => self.open_sheet(),
            KeyCode::Char('n') => self.jump_match(1),
            KeyCode::Char('N') => self.jump_match(-1),
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            _ => {
                if let View::Conversation { scroll, .. } = &mut self.view {
                    *scroll = match key.code {
                        KeyCode::Down | KeyCode::Char('j') => *scroll + 1,
                        KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
                        KeyCode::PageDown | KeyCode::Char(' ') => *scroll + page,
                        KeyCode::PageUp => scroll.saturating_sub(page),
                        KeyCode::Char('g') | KeyCode::Home => 0,
                        KeyCode::Char('G') | KeyCode::End => max,
                        _ => *scroll,
                    }
                    .min(max);
                }
            }
        }
    }

    fn palette_key(&mut self, key: KeyEvent, ctrl: bool) {
        let sugg = self.loc_suggestions().len();
        let completing = self.completing().is_some() && sugg > 0;
        let mv = |app: &mut App, d: isize| {
            if completing {
                app.sel = (app.sel as isize + d).clamp(0, sugg as isize - 1) as usize;
            } else {
                app.move_sel(d);
            }
        };
        match key.code {
            KeyCode::Char(c) if ctrl => match c {
                'u' => {
                    self.query.clear();
                    self.recompute();
                }
                'x' => self.toggle_hidden(),
                't' => self.toggle_tools(),
                'o' => self.open_conversation(),
                'n' | 'j' => mv(self, 1),
                'p' | 'k' => mv(self, -1),
                'l' => {
                    if let Some(l) = self.launch_location() {
                        let q = self.query.trim_end().to_string();
                        self.query = format!("{q}{}@{l} ", if q.is_empty() { "" } else { " " });
                        self.recompute();
                        self.last = "this location".into();
                    }
                }
                _ => {}
            },
            KeyCode::Char(c) => {
                self.query.push(c);
                self.recompute();
                self.sel = 0;
                self.last = format!("query “{}”", self.query);
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute();
                self.sel = 0;
            }
            KeyCode::Esc => {
                if self.query.is_empty() {
                    self.quit = true;
                } else {
                    self.query.clear();
                    self.recompute();
                    self.last = "query cleared".into();
                }
            }
            KeyCode::Up => mv(self, -1),
            KeyCode::Down => mv(self, 1),
            KeyCode::PageUp => mv(self, -self.page()),
            KeyCode::PageDown => mv(self, self.page()),
            KeyCode::Tab | KeyCode::Enter if completing => self.complete(),
            KeyCode::Tab => {
                // Open the location browser.
                if !self.query.is_empty() && !self.query.ends_with(' ') {
                    self.query.push(' ');
                }
                self.query.push('@');
                self.sel = 0;
                self.recompute();
                self.last = "browse locations".into();
            }
            KeyCode::Enter => self.open_sheet(),
            KeyCode::Right => self.open_conversation(),
            KeyCode::F(1) => self.help = true,
            _ => {}
        }
    }
}
