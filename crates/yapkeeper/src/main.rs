//! The `yapkeeper` command line. It owns presentation only: one plain line per
//! session or path for people, `grep`, `cut` and `wc -l`, and diagnostics and
//! confirmations on stderr.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use yapkeeper::{Availability, DataDir, IndexError, Listed, Place, Roots, list, rebuild};

/// Wide enough for most titles; what follows may wrap on an 80-column terminal.
const MAX_TITLE_WIDTH: usize = 48;

const EXIT_FAILURE: u8 = 1;

const HISTORIES: &str = "Histories are read, never modified, from:
  Claude Code  $CLAUDE_CONFIG_DIR/projects (default ~/.claude/projects)
  Codex        $CODEX_HOME (default ~/.codex)";

/// Find coding sessions across projects, worktrees and folders.
#[derive(Parser)]
#[command(version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List Claude Code and Codex sessions, most recent first
    ///
    /// Each line shows a session's title, provider, age, an orphaned or unavailable
    /// label when one applies, and where it ran.
    List,
    /// Print where Yapkeeper keeps its index and preferences
    Paths,
    /// Delete the index and build it again from every history file
    Rebuild,
}

fn main() -> ExitCode {
    // Built at runtime so help names the files this environment resolves to.
    let footer = help_footer();
    let matches = Cli::command()
        .after_help(footer.clone())
        .mut_subcommands(|subcommand| subcommand.after_help(footer.clone()))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    match cli.command {
        Command::List => print_list(),
        Command::Paths => print_paths(),
        Command::Rebuild => print_rebuild(),
    }
}

/// Where histories are read from and where Yapkeeper keeps its own files,
/// so nothing it stores is hidden.
fn help_footer() -> String {
    let own_files = match DataDir::from_env() {
        Ok(data) => format!(
            "  Index        {} (rebuildable)\n  Preferences  {}",
            data.index().display(),
            data.prefs().display()
        ),
        Err(_) => {
            "  Folder       $XDG_DATA_HOME/yapkeeper (default ~/.local/share/yapkeeper)".to_string()
        }
    };
    format!("{HISTORIES}\n\nYapkeeper's own files:\n{own_files}")
}

/// Two space-aligned columns, so `cut` and `awk` can read them. Creates nothing.
fn print_paths() -> ExitCode {
    let data = match DataDir::from_env() {
        Ok(data) => data,
        Err(error) => {
            eprintln!("yapkeeper: {error}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    println!("index        {}", data.index().display());
    println!("preferences  {}", data.prefs().display());
    ExitCode::SUCCESS
}

/// The history roots and data directory this environment names, or the
/// exit code once it has said why there are none.
fn environment() -> Result<(Roots, DataDir), ExitCode> {
    match (Roots::from_env(), DataDir::from_env()) {
        (Ok(roots), Ok(data)) => Ok((roots, data)),
        (Err(error), _) | (_, Err(error)) => {
            eprintln!("yapkeeper: {error}");
            Err(ExitCode::from(EXIT_FAILURE))
        }
    }
}

/// Says why the index failed and how to fix it, and gives the exit code.
fn index_failure(error: IndexError) -> ExitCode {
    eprintln!("yapkeeper: {error}");
    eprintln!("Check its permissions, or set XDG_DATA_HOME.");
    ExitCode::from(EXIT_FAILURE)
}

/// The confirmation goes to stderr, so stdout stays empty for scripts.
fn print_rebuild() -> ExitCode {
    let (roots, data) = match environment() {
        Ok(environment) => environment,
        Err(code) => return code,
    };
    match rebuild(&roots, &data) {
        Ok(listing) => {
            let sessions = match listing.sessions.len() {
                1 => "1 session".to_string(),
                n => format!("{n} sessions"),
            };
            eprintln!(
                "yapkeeper: rebuilt the index at {} ({sessions})",
                data.index().display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => index_failure(error),
    }
}

fn print_list() -> ExitCode {
    let (roots, data) = match environment() {
        Ok(environment) => environment,
        Err(code) => return code,
    };
    let listing = match list(&roots, &data) {
        Ok(listing) => listing,
        Err(error) => return index_failure(error),
    };
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
        .map(|listed| fit_title(&one_line(&listed.session.title)))
        .collect();
    let title_width = titles.iter().map(|title| title.width()).max().unwrap_or(0);
    let provider_width = listing
        .sessions
        .iter()
        .map(|listed| listed.session.provider.width())
        .max()
        .unwrap_or(0);
    let state_width = listing
        .sessions
        .iter()
        .map(|listed| state_label(listed.location.availability).width())
        .max()
        .unwrap_or(0);
    let mut stdout = io::stdout().lock();
    for (listed, title) in listing.sessions.iter().zip(&titles) {
        let session = &listed.session;
        let padding = " ".repeat(title_width - title.width());
        let provider = session.provider;
        let age = ago(session.last_active, now);
        // The label column is dropped when no line has one, rather than
        // left as blank padding.
        let state = match state_width {
            0 => String::new(),
            _ => format!(
                "{:<state_width$}  ",
                state_label(listed.location.availability)
            ),
        };
        let location = without_controls(&location_text(listed, home.as_deref()));
        // A closed pipe, as with `| head`, just ends the listing.
        if writeln!(
            stdout,
            "{title}{padding}  {provider:<provider_width$}  {age:>3}  {state}{location}"
        )
        .is_err()
        {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// Words rather than colour, so the state survives `NO_COLOR` and `grep`.
fn state_label(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "",
        Availability::Orphaned => "orphaned",
        Availability::Unavailable => "unavailable",
    }
}

/// `project`, `project › worktree`, or the working directory when the
/// session ran outside any known repo.
fn location_text(listed: &Listed, home: Option<&Path>) -> String {
    match &listed.location.place {
        Place::MainCheckout { project } => project.clone(),
        Place::Worktree { project, worktree } => format!("{project} › {worktree}"),
        Place::Folder => with_tilde(&listed.session.working_dir, home),
    }
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
