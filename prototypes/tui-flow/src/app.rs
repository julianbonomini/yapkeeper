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

    /// Free-text terms; filter tokens (see `parse`) are excluded.
    pub fn terms(&self) -> Vec<String> {
        parse(&self.query).0
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

    fn filter_ok(s: &Session, f: &Filter) -> bool {
        match f {
            Filter::Loc(loc) => format!("{} {} {}", s.loc_key(), s.location(), s.dir())
                .to_ascii_lowercase()
                .contains(loc.as_str()),
            Filter::Agent(a) => s.provider.name().starts_with(a.as_str()),
            Filter::State(st) => match st.as_str() {
                "hidden" => s.hidden,
                "orphaned" | "orphan" => s.availability == Availability::Orphaned,
                "unavailable" => s.availability == Availability::Unknown,
                "temp" => s.is_temp(),
                "pr" => !s.prs.is_empty(),
                "recap" => s.recap.is_some(),
                "git" | "repo" => s.project.is_some(),
                "folder" => s.project.is_none(),
                _ => true,
            },
        }
    }

    /// All filters must pass. Hidden sessions appear only when asked for with `is hidden`.
    fn filters_ok(&self, s: &Session) -> bool {
        let (_, filters) = parse(&self.query);
        let asks_hidden = filters
            .iter()
            .any(|(neg, f)| !neg && *f == Filter::State("hidden".into()));
        filters.iter().all(|(neg, f)| Self::filter_ok(s, f) != *neg) && (!s.hidden || asks_hidden)
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

    /// What the cursor is completing, if anything: kind and the partial text.
    pub fn completing(&self) -> Option<(Kind, String)> {
        let toks: Vec<&str> = self.query.split_whitespace().collect();
        let ended = self.query.ends_with(' ') || self.query.is_empty();
        let (prev, cur) = if ended {
            (toks.last().copied(), None)
        } else {
            (
                toks.len().checked_sub(2).map(|i| toks[i]),
                toks.last().copied(),
            )
        };
        let natural = matches!(prev, Some("is" | "not" | "!" | "!is"));
        match cur {
            None if natural => Some((Kind::State, String::new())),
            None => None,
            Some(c) => {
                let c = c.trim_start_matches(['-', '!']).to_ascii_lowercase();
                if let Some(p) = c.strip_prefix('@') {
                    Some((Kind::Loc, p.to_string()))
                } else if let Some(p) = c.strip_prefix('#') {
                    Some((Kind::Agent, p.to_string()))
                } else if let Some(p) = c.strip_prefix("is:") {
                    Some((Kind::State, p.to_string()))
                } else if natural {
                    Some((Kind::State, c))
                } else {
                    None
                }
            }
        }
    }

    /// Completions for the current token, filtered by the partial text.
    pub fn suggestions(&self) -> Vec<Sugg> {
        let Some((kind, part)) = self.completing() else {
            return vec![];
        };
        let visible = || self.sessions.iter().filter(|s| !s.hidden);
        let count = |f: &Filter| visible().filter(|s| Self::filter_ok(s, f)).count();
        match kind {
            Kind::Agent => ["claude", "codex"]
                .iter()
                .filter(|a| a.starts_with(part.as_str()))
                .map(|a| Sugg {
                    token: format!("#{a}"),
                    label: a.to_string(),
                    detail: String::new(),
                    count: count(&Filter::Agent(a.to_string())),
                    depth: 0,
                })
                .collect(),
            Kind::State => [
                ("orphaned", "folder no longer exists"),
                ("temp", "in a system temp folder"),
                ("pr", "linked to a pull request"),
                ("recap", "has a recap"),
                ("git", "in a Git repo or worktree"),
                ("folder", "in a plain folder, not Git"),
                ("unavailable", "folder can't be checked"),
                ("hidden", "hidden by you"),
            ]
            .iter()
            .filter(|(st, _)| st.starts_with(part.as_str()))
            .map(|(st, why)| Sugg {
                token: st.to_string(),
                label: st.to_string(),
                detail: why.to_string(),
                count: if *st == "hidden" {
                    self.sessions.iter().filter(|s| s.hidden).count()
                } else {
                    count(&Filter::State(st.to_string()))
                },
                depth: 0,
            })
            .collect(),
            Kind::Loc => self.loc_suggestions(&part),
        }
    }

    /// Projects first with their worktrees nested beneath, then folders; groups ordered
    /// by most recent activity. Matches the label or anywhere in the path.
    fn loc_suggestions(&self, part: &str) -> Vec<Sugg> {
        // key -> (group, depth, label, path, count, recency, every session orphaned)
        let mut rows: Vec<LocRow> = Vec::new();
        for s in self.sessions.iter().filter(|s| !s.hidden) {
            let mut entries = vec![];
            match (&s.project, &s.branch) {
                (Some(p), b) => {
                    let main_root = if b.is_none() {
                        s.root.clone()
                    } else {
                        String::new()
                    };
                    entries.push((p.clone(), p.clone(), 0, p.clone(), main_root));
                    if let Some(b) = b {
                        entries.push((format!("{p}:{b}"), p.clone(), 1, b.clone(), s.root.clone()));
                    }
                }
                _ => entries.push((
                    s.root.clone(),
                    s.root.clone(),
                    0,
                    s.root.clone(),
                    String::new(),
                )),
            }
            let gone = s.availability == Availability::Orphaned;
            for (key, group, depth, label, path) in entries {
                match rows.iter_mut().find(|r| r.0 == key) {
                    Some(r) => {
                        r.7 &= gone;
                        r.5 += 1;
                        r.6 = r.6.min(s.minutes_ago);
                        if r.4.is_empty() {
                            r.4 = path;
                        }
                    }
                    None => rows.push((key, group, depth, label, path, 1, s.minutes_ago, gone)),
                }
            }
        }
        rows.retain(|r| {
            format!("{} {}", r.0, r.4)
                .to_ascii_lowercase()
                .contains(part)
        });
        let group_recency = |g: &str| rows.iter().filter(|r| r.1 == g).map(|r| r.6).min();
        let worktrees = |g: &str| rows.iter().filter(|r| r.1 == g && r.2 == 1).count();
        let mut keyed: Vec<_> = rows
            .iter()
            .map(|r| ((group_recency(&r.1), r.1.clone(), r.2, r.6), r))
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed
            .into_iter()
            .map(|(_, r)| {
                let n = worktrees(&r.1);
                // Lead with what the location is: a Git repo, one of its worktrees, or a
                // plain folder. Folders that no longer exist can't be classified.
                let detail = match (r.2, n) {
                    (1, _) => format!("worktree · {}", r.4),
                    (_, 0) if r.1 == r.0 && r.3 == r.0 && r.7 => {
                        "missing · folder no longer exists".into()
                    }
                    (_, 0) if r.3 == r.0 && r.4.is_empty() => "folder".into(),
                    (_, 0) => format!("git repo · {}", r.4),
                    (_, n) => {
                        let wt =
                            format!("git repo · {n} worktree{}", if n == 1 { "" } else { "s" });
                        if r.4.is_empty() {
                            wt
                        } else {
                            format!("{wt} · {}", r.4)
                        }
                    }
                };
                Sugg {
                    token: format!("@{}", r.0),
                    label: r.3.clone(),
                    detail,
                    count: r.5,
                    depth: r.2,
                }
            })
            .collect()
    }

    fn complete(&mut self) {
        let sugg = self.suggestions();
        let Some(pick) = sugg.get(self.sel) else {
            return;
        };
        let Some((kind, _)) = self.completing() else {
            return;
        };
        let ended = self.query.ends_with(' ');
        let mut toks: Vec<String> = self.query.split_whitespace().map(str::to_string).collect();
        let neg = !ended && toks.last().is_some_and(|t| t.starts_with(['-', '!']));
        let natural = {
            let prev = if ended {
                toks.last()
            } else {
                toks.iter().rev().nth(1)
            };
            matches!(prev.map(String::as_str), Some("is" | "not" | "!" | "!is"))
        };
        if !ended {
            toks.pop();
        }
        let token = match kind {
            Kind::State if !natural => format!("is:{}", pick.token),
            _ => pick.token.clone(),
        };
        toks.push(format!("{}{token}", if neg { "-" } else { "" }));
        self.query = format!("{} ", toks.join(" "));
        self.sel = 0;
        self.recompute();
        self.last = format!("filter → {token}");
    }

    fn launch_location(&self) -> Option<String> {
        self.sessions
            .iter()
            .filter(|s| !s.root.is_empty() && under(&self.cwd, &s.root))
            .max_by_key(|s| s.root.len())
            .map(Session::loc_key)
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
        let sugg = self.suggestions().len();
        let completing = sugg > 0;
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

/// (key, group, depth, label, path, sessions, recency, every session orphaned)
type LocRow = (String, String, u16, String, String, usize, u32, bool);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Loc,
    Agent,
    State,
}

pub struct Sugg {
    /// Text inserted into the query (state words without their `is:` prefix).
    pub token: String,
    pub label: String,
    pub detail: String,
    pub count: usize,
    pub depth: u16,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Filter {
    Loc(String),
    Agent(String),
    State(String),
}

const STATES: &[&str] = &[
    "orphaned",
    "orphan",
    "temp",
    "pr",
    "recap",
    "git",
    "repo",
    "folder",
    "unavailable",
    "hidden",
];

/// Split a query into free-text terms and filters. Filters: `@loc`, `#agent`, `is:state`,
/// the natural forms `is orphaned` / `not codex`, and `not`, `!`, or `-` to negate.
pub fn parse(q: &str) -> (Vec<String>, Vec<(bool, Filter)>) {
    let toks: Vec<&str> = q.split_whitespace().collect();
    let mut terms = vec![];
    let mut filters = vec![];
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i];
        let low = tok.to_ascii_lowercase();
        let next = toks.get(i + 1).map(|t| t.to_ascii_lowercase());
        // Natural form: `is orphaned`, `not temp`, `not codex`, `not @scratch`.
        if matches!(low.as_str(), "is" | "not" | "!" | "!is")
            && let Some(n) = &next
        {
            let neg = low != "is";
            let f = if STATES.contains(&n.as_str()) {
                Some(Filter::State(n.clone()))
            } else if ["claude", "codex"].contains(&n.as_str()) {
                Some(Filter::Agent(n.clone()))
            } else {
                token_filter(n)
            };
            if let Some(f) = f {
                filters.push((neg, f));
                i += 2;
                continue;
            }
        }
        let (neg, body) = match low.strip_prefix(['-', '!']) {
            Some(b) if b.len() > 1 => (true, b.to_string()),
            _ => (false, low.clone()),
        };
        match token_filter(&body) {
            Some(f) => filters.push((neg, f)),
            None => terms.push(tok.to_string()),
        }
        i += 1;
    }
    (terms, filters)
}

fn token_filter(t: &str) -> Option<Filter> {
    if let Some(l) = t.strip_prefix('@') {
        Some(Filter::Loc(l.to_string()))
    } else if let Some(a) = t.strip_prefix('#') {
        Some(Filter::Agent(a.to_string()))
    } else {
        t.strip_prefix("is:")
            .map(|st| Filter::State(st.to_string()))
    }
}
