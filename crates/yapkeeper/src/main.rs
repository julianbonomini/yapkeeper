//! The `yapkeeper` command line. It owns presentation only: one plain line per
//! session for people, `grep` and `wc -l`, and diagnostics on stderr.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Parser, Subcommand};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use yapkeeper::{Roots, list};

/// Wide enough for most titles while leaving the directory room at 80 columns.
const MAX_TITLE_WIDTH: usize = 48;

const EXIT_MISSING_HOME: u8 = 1;

const HISTORIES: &str = "Histories are read, never modified, from:
  Claude Code  $CLAUDE_CONFIG_DIR/projects (default ~/.claude/projects)
  Codex        $CODEX_HOME (default ~/.codex)";

/// Find coding sessions across projects, worktrees and folders.
#[derive(Parser)]
#[command(version, arg_required_else_help = true, after_help = HISTORIES)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List Claude Code and Codex sessions, most recent first
    ///
    /// Each line shows a session's title, provider, age and working directory.
    #[command(after_help = HISTORIES)]
    List,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::List => print_list(),
    }
}

fn print_list() -> ExitCode {
    let roots = match Roots::from_env() {
        Ok(roots) => roots,
        Err(error) => {
            eprintln!("yapkeeper: {error}");
            return ExitCode::from(EXIT_MISSING_HOME);
        }
    };
    let listing = list(&roots);
    if listing.sessions.is_empty() {
        for dir in &listing.searched {
            eprintln!("yapkeeper: no sessions found in {}", dir.display());
        }
    }
    match listing.malformed_lines {
        0 => {}
        1 => eprintln!("yapkeeper: skipped 1 malformed line"),
        n => eprintln!("yapkeeper: skipped {n} malformed lines"),
    }
    let now = SystemTime::now();
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    let titles: Vec<String> = listing
        .sessions
        .iter()
        .map(|session| fit_title(&one_line(&session.title)))
        .collect();
    let title_width = titles.iter().map(|title| title.width()).max().unwrap_or(0);
    let provider_width = listing
        .sessions
        .iter()
        .map(|session| session.provider.width())
        .max()
        .unwrap_or(0);
    let mut stdout = io::stdout().lock();
    for (session, title) in listing.sessions.iter().zip(&titles) {
        let padding = " ".repeat(title_width - title.width());
        let provider = session.provider;
        let age = ago(session.last_active, now);
        let dir = without_controls(&with_tilde(&session.working_dir, home.as_deref()));
        // A closed pipe, as with `| head`, just ends the listing.
        if writeln!(
            stdout,
            "{title}{padding}  {provider:<provider_width$}  {age:>3}  {dir}"
        )
        .is_err()
        {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// `dir` with a leading home directory shown as `~`.
fn with_tilde(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

/// Transcript text on one line: control characters such as ESC are removed,
/// then each whitespace run, newlines included, becomes one space.
fn one_line(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| c.is_whitespace() || !c.is_control())
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Recorded text can't move the cursor or restyle the terminal once control
/// characters such as ESC are gone.
fn without_controls(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The title cut to at most `MAX_TITLE_WIDTH` terminal cells, ending in `…`
/// when cut.
fn fit_title(title: &str) -> String {
    if title.width() <= MAX_TITLE_WIDTH {
        return title.to_string();
    }
    let mut fitted = String::new();
    let mut width = 0;
    for c in title.chars() {
        let char_width = c.width().unwrap_or(0);
        if width + char_width > MAX_TITLE_WIDTH - 1 {
            break;
        }
        fitted.push(c);
        width += char_width;
    }
    fitted.push('…');
    fitted
}

/// How long ago `then` was, in the largest unit that keeps the number
/// readable: `42m`, `5h`, `9d`, `30w`. A time in the future shows `0m`.
fn ago(then: SystemTime, now: SystemTime) -> String {
    let minutes = now
        .duration_since(then)
        .map(|elapsed| elapsed.as_secs() / 60)
        .unwrap_or(0);
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 14 => format!("{}d", m / (60 * 24)),
        m => format!("{}w", m / (60 * 24 * 7)),
    }
}
