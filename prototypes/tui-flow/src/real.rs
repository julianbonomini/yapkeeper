//! PROTOTYPE loader for real Claude Code and Codex histories. Read-only: it opens
//! files for reading and never writes, renames, or deletes anything under them.
//! Tool results are skipped and messages are capped to keep memory small.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};

use serde_json::Value;

use crate::data::{Availability, Msg, Provider, Role, Session};

const MAX_MSG_CHARS: usize = 4000;
const MAX_TOOL_CHARS: usize = 300;

#[derive(Default, Debug)]
pub struct Stats {
    pub files: usize,
    pub bytes: u64,
    pub sessions: usize,
    pub empty: usize,
    pub subagent_skipped: usize,
    pub bad_lines: usize,
    pub orphaned: usize,
    pub unavailable: usize,
    pub locations: usize,
    pub ms: u128,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `/Users/me/x` → `~/x` for display.
pub fn tilde(p: &str) -> String {
    let h = home();
    let h = h.to_string_lossy();
    match p.strip_prefix(h.as_ref()) {
        Some(rest) if !h.is_empty() => format!("~{rest}"),
        _ => p.to_string(),
    }
}

fn cap(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn minutes_since(path: &Path) -> u32 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| (d.as_secs() / 60) as u32)
        .unwrap_or(u32::MAX / 2)
}

/// Injected context rather than something the person typed.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("Caveat:") || t.starts_with("# AGENTS.md")
}

fn blank(provider: Provider, id: String, path: &Path) -> Session {
    Session {
        id,
        provider,
        title: None,
        project: None,
        branch: None,
        root: String::new(),
        subdir: String::new(),
        minutes_ago: minutes_since(path),
        availability: Availability::Resumable,
        hidden: false,
        subagents: vec![],
        msgs: vec![],
    }
}

fn push(s: &mut Session, role: Role, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let n = if role == Role::Tool {
        MAX_TOOL_CHARS
    } else {
        MAX_MSG_CHARS
    };
    s.msgs.push(Msg {
        role,
        text: cap(text, n),
    });
}

/// Returns (session, cwd, bad lines). `None` session means skip.
fn parse_claude(path: &Path) -> (Option<Session>, Option<String>, usize) {
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut s = blank(Provider::Claude, id, path);
    let Ok(f) = fs::File::open(path) else {
        return (None, None, 0);
    };
    let mut cwd = None;
    let mut custom = None;
    let mut ai = None;
    let mut bad = 0;
    for line in BufReader::new(f).lines() {
        let Ok(line) = line else { break };
        let msg = line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"");
        let title = line.contains("custom-title") || line.contains("ai-title");
        if !msg && !title {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            bad += 1;
            continue;
        };
        match v["type"].as_str() {
            Some("custom-title") => custom = v["customTitle"].as_str().map(str::to_string),
            Some("ai-title") => ai = v["aiTitle"].as_str().map(str::to_string),
            Some(kind @ ("user" | "assistant")) => {
                if v["isMeta"].as_bool() == Some(true) || v["isSidechain"].as_bool() == Some(true) {
                    continue;
                }
                if cwd.is_none() {
                    cwd = v["cwd"].as_str().map(str::to_string);
                }
                let role = if kind == "user" {
                    Role::User
                } else {
                    Role::Assistant
                };
                match &v["message"]["content"] {
                    Value::String(t) if !(role == Role::User && is_injected(t)) => {
                        push(&mut s, role, t)
                    }
                    Value::Array(blocks) => {
                        for b in blocks {
                            match b["type"].as_str() {
                                Some("text") => {
                                    let t = b["text"].as_str().unwrap_or("");
                                    if !(role == Role::User && is_injected(t)) {
                                        push(&mut s, role, t);
                                    }
                                }
                                Some("tool_use") => {
                                    let name = b["name"].as_str().unwrap_or("tool");
                                    push(&mut s, Role::Tool, &format!("{name} {}", b["input"]));
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    s.title = custom.or(ai);
    // Subagent transcripts live beside the session file; count them only.
    let sub_dir = path.with_extension("").join("subagents");
    if let Ok(rd) = fs::read_dir(sub_dir) {
        s.subagents = rd.filter_map(Result::ok).map(|_| vec![]).collect();
    }
    if s.msgs.iter().all(|m| m.role != Role::User) {
        return (None, cwd, bad);
    }
    (Some(s), cwd, bad)
}

/// Returns (session, cwd, bad lines, is_subagent).
fn parse_codex(path: &Path) -> (Option<Session>, Option<String>, usize, bool) {
    let mut s = blank(Provider::Codex, String::new(), path);
    let Ok(f) = fs::File::open(path) else {
        return (None, None, 0, false);
    };
    let mut cwd = None;
    let mut bad = 0;
    let mut meta_seen = false;
    for line in BufReader::new(f).lines() {
        let Ok(line) = line else { break };
        let meta = !meta_seen && line.contains("\"session_meta\"");
        let item = line.contains("\"response_item\"")
            && !line.contains("\"type\":\"reasoning\"")
            && !line.contains("_output\"");
        if !meta && !item {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            bad += 1;
            continue;
        };
        let p = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") => {
                meta_seen = true;
                if p["source"].get("subagent").is_some() {
                    return (None, None, bad, true);
                }
                s.id = p["id"].as_str().unwrap_or_default().to_string();
                cwd = p["cwd"].as_str().map(str::to_string);
            }
            Some("response_item") => match p["type"].as_str() {
                Some("message") => {
                    let role = match p["role"].as_str() {
                        Some("user") => Role::User,
                        Some("assistant") => Role::Assistant,
                        _ => continue,
                    };
                    for b in p["content"].as_array().into_iter().flatten() {
                        let t = b["text"].as_str().unwrap_or("");
                        if !(role == Role::User && is_injected(t)) {
                            push(&mut s, role, t);
                        }
                    }
                }
                Some("function_call") => {
                    let name = p["name"].as_str().unwrap_or("tool");
                    let args = p["arguments"].as_str().unwrap_or("");
                    push(&mut s, Role::Tool, &format!("{name} {args}"));
                }
                Some("custom_tool_call") => {
                    let name = p["name"].as_str().unwrap_or("tool");
                    let input = p["input"].as_str().unwrap_or("");
                    push(&mut s, Role::Tool, &format!("{name} {input}"));
                }
                _ => {}
            },
            _ => {}
        }
    }
    if s.id.is_empty() || s.msgs.iter().all(|m| m.role != Role::User) {
        return (None, cwd, bad, false);
    }
    (Some(s), cwd, bad, false)
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth > 0 {
            walk(&p, depth - 1, out, keep);
        } else if ft.is_file() && keep(&p) {
            out.push(p);
        }
    }
}

struct Place {
    project: Option<String>,
    branch: Option<String>,
    root: String,
    availability: Availability,
}

/// Classify a recorded working directory: Git main worktree, linked worktree, plain
/// folder, orphaned (gone), or unavailable (can't inspect).
fn place(cwd: &str) -> Place {
    let path = Path::new(cwd);
    let availability = match fs::metadata(path) {
        Ok(_) => Availability::Resumable,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Availability::Orphaned,
        Err(_) => Availability::Unknown,
    };
    let plain = Place {
        project: None,
        branch: None,
        root: cwd.to_string(),
        availability,
    };
    if availability != Availability::Resumable {
        return plain;
    }
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    for dir in path.ancestors() {
        let git = dir.join(".git");
        let Ok(meta) = fs::metadata(&git) else {
            continue;
        };
        let root = dir.to_string_lossy().into_owned();
        if meta.is_dir() {
            return Place {
                project: name(dir),
                branch: None,
                root,
                availability,
            };
        }
        // Linked worktree: `.git` is a file pointing at `<main>/.git/worktrees/<name>`.
        let main = fs::read_to_string(&git).ok().and_then(|t| {
            let gitdir = t.trim().strip_prefix("gitdir:")?.trim().to_string();
            let gitdir = PathBuf::from(gitdir);
            let main_git = gitdir.parent()?.parent()?.to_path_buf();
            main_git.parent().map(Path::to_path_buf)
        });
        return Place {
            project: main.as_deref().and_then(name).or_else(|| name(dir)),
            branch: name(dir),
            root,
            availability,
        };
    }
    plain
}

pub fn load() -> (Vec<Session>, Stats) {
    let started = Instant::now();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude"));
    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"));
    let jsonl = |p: &Path| p.extension().is_some_and(|e| e == "jsonl");

    let mut files: Vec<(Provider, PathBuf)> = Vec::new();
    let mut found = Vec::new();
    // projects/<encoded>/<id>.jsonl only; deeper files are subagent transcripts.
    walk(&claude.join("projects"), 1, &mut found, &jsonl);
    files.extend(found.drain(..).map(|p| (Provider::Claude, p)));
    walk(&codex.join("sessions"), 4, &mut found, &jsonl);
    walk(&codex.join("archived_sessions"), 1, &mut found, &jsonl);
    files.extend(found.drain(..).map(|p| (Provider::Codex, p)));

    let mut stats = Stats {
        files: files.len(),
        bytes: files
            .iter()
            .filter_map(|(_, p)| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum(),
        ..Stats::default()
    };

    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(Session, Option<String>)>> = Mutex::new(Vec::new());
    let counters = Mutex::new((0usize, 0usize, 0usize)); // empty, subagent, bad
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((provider, path)) = files.get(i) else {
                        break;
                    };
                    let (session, cwd, bad, sub) = match provider {
                        Provider::Claude => {
                            let (s, c, b) = parse_claude(path);
                            (s, c, b, false)
                        }
                        Provider::Codex => parse_codex(path),
                    };
                    let mut c = counters.lock().unwrap();
                    c.2 += bad;
                    match session {
                        Some(s) => out.lock().unwrap().push((s, cwd)),
                        None if sub => c.1 += 1,
                        None => c.0 += 1,
                    }
                }
            });
        }
    });
    let (empty, sub, bad) = *counters.lock().unwrap();
    stats.empty = empty;
    stats.subagent_skipped = sub;
    stats.bad_lines = bad;

    // Codex titles come from the session index.
    let mut titles = HashMap::new();
    if let Ok(t) = fs::read_to_string(codex.join("session_index.jsonl")) {
        for line in t.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let (Some(id), Some(name)) = (v["id"].as_str(), v["thread_name"].as_str())
            {
                titles.insert(id.to_string(), name.to_string());
            }
        }
    }

    let mut places: HashMap<String, Place> = HashMap::new();
    let mut sessions = Vec::new();
    for (mut s, cwd) in out.into_inner().unwrap() {
        let cwd = cwd.unwrap_or_default();
        if s.provider == Provider::Codex {
            s.title = titles.get(&s.id).cloned();
        }
        if !cwd.is_empty() {
            let pl = places.entry(cwd.clone()).or_insert_with(|| place(&cwd));
            s.project = pl.project.clone();
            s.branch = pl.branch.clone();
            s.availability = pl.availability;
            s.subdir = cwd[pl.root.len().min(cwd.len())..].to_string();
            s.root = tilde(&pl.root);
        } else {
            s.availability = Availability::Unknown;
            s.root = "(no directory recorded)".into();
        }
        match s.availability {
            Availability::Orphaned => stats.orphaned += 1,
            Availability::Unknown => stats.unavailable += 1,
            Availability::Resumable => {}
        }
        sessions.push(s);
    }
    stats.sessions = sessions.len();
    stats.locations = places.len();
    stats.ms = started.elapsed().as_millis();
    (sessions, stats)
}
