//! The `yapkeeper` command line. It owns presentation only: one plain line per
//! session or path for people, `grep`, `cut` and `wc -l`, and diagnostics and
//! confirmations on stderr.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use yapkeeper::{
    Availability, DataDir, DataError, Listed, MissingHome, Place, Query, Roots, State, TokenKind,
    hide, rebuild, search, unhide,
};

/// Wide enough for most titles; what follows may wrap on an 80-column terminal.
const MAX_TITLE_WIDTH: usize = 48;

const EXIT_FAILURE: u8 = 1;

/// Find coding sessions wherever they ran: in a repo, a worktree or a folder.
#[derive(Parser)]
#[command(version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List every session, most recent first
    ///
    /// Each line shows a session's title, provider, age, an orphaned or unavailable
    /// label when one applies, and where it ran. Sessions in temp folders are left
    /// out, as are hidden ones; yapkeeper search is:temp and is:hidden list them.
    /// Orphaned sessions come after the rest, each group most recent first.
    List(Columns),
    /// Find sessions by what was said, where they ran and their state
    Search(Search),
    /// Hide a session from list and search; search is:hidden still lists it
    ///
    /// The hide is kept in the preferences, so rebuilding the index keeps it.
    /// The provider's history is never touched.
    Hide(Id),
    /// List a hidden session again
    Unhide(Id),
    /// Print where Yapkeeper keeps its index and preferences
    Paths,
    /// Delete the index and build it again from every history file
    Rebuild,
}

/// What each line shows beyond the default columns.
#[derive(Args, Default)]
struct Columns {
    /// Start each line with the session's id, as hide and unhide take it
    #[arg(long)]
    ids: bool,
}

/// What to search for; the default finds every session, as `list` does.
#[derive(Args, Default)]
struct Search {
    #[command(flatten)]
    columns: Columns,
    /// Also search tool calls, tool output and subagent transcripts
    #[arg(long)]
    tools: bool,
    /// Words and filters every listed session matches
    #[arg(num_args = 0..)]
    query: Vec<String>,
}

#[derive(Args)]
struct Id {
    /// The session's id, from yapkeeper list --ids
    id: String,
}

fn main() -> ExitCode {
    // Built at runtime so help names what this environment resolves to.
    let roots = Roots::from_env();
    let providers = roots.as_ref().map(Roots::providers).unwrap_or_default();
    let footer = help_footer(roots.as_ref());
    let matches = Cli::command()
        .mut_subcommand("search", |search| {
            // `long_about` replaces `about` in `--help`, so it starts with it.
            let about = search.get_about().map(ToString::to_string);
            let about = about.unwrap_or_default();
            search.long_about(format!("{about}\n\n{}", search_help(&providers)))
        })
        .after_help(footer.clone())
        .mut_subcommands(|subcommand| subcommand.after_help(footer.clone()))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    match cli.command {
        Command::List(columns) => print_sessions(&Search {
            columns,
            ..Search::default()
        }),
        Command::Search(search) => print_sessions(&search),
        Command::Hide(Id { id }) => print_hidden(&id, hide),
        Command::Unhide(Id { id }) => print_hidden(&id, unhide),
        Command::Paths => print_paths(),
        Command::Rebuild => print_rebuild(),
    }
}

/// The query language as `search --help` explains it, naming every provider
/// in `providers` and every state.
fn search_help(providers: &[&str]) -> String {
    let temp = State::Temp.name();
    let hidden = State::Hidden.name();
    let providers: Vec<String> = providers.iter().map(|p| format!("#{p}")).collect();
    let provider_sentence = match providers.is_empty() {
        true => String::new(),
        false => format!(" #provider matches {}.", or_list(&providers)),
    };
    let states: Vec<String> = State::ALL
        .iter()
        .map(|state| format!("is:{}", state.name()))
        .collect();
    let states = or_list(&states);
    format!(
        "Words match titles, messages and recaps as substrings; every word must \
         match. A word with an uppercase letter matches case. --tools also \
         searches tool calls, tool output and subagent transcripts. Put -- before \
         a word that starts with -.\n\n\
         Filters must all match too. @where matches a project, project:worktree or \
         any part of the path.{provider_sentence} is:state matches {states}. !x or \
         not:x excludes a filter, a provider or state named bare (!{temp}) or a \
         word (!flaky); quote ! for the shell, as in '!{temp}'. A space ends a \
         filter, so \"is pr\" is two words.\n\n\
         Sessions in temp folders are left out unless the query has is:{temp}, \
         and hidden ones unless it has is:{hidden}. Orphaned sessions come after the \
         rest, each group most recent first."
    )
}

/// `a`, `a or b`, `a, b or c`: no serial comma, as the rest of help writes it.
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Where histories are read from and where Yapkeeper keeps its own files,
/// so nothing it stores is hidden. Every path starts in one column; a
/// history directory the environment can't resolve is replaced by why.
fn help_footer(roots: Result<&Roots, &MissingHome>) -> String {
    let histories = roots.map(|roots| {
        roots
            .history_dirs()
            .into_iter()
            .map(|history_dir| FooterRow {
                label: history_dir.provider,
                path: history_dir.dir.display().to_string(),
            })
            .collect::<Vec<_>>()
    });
    let own_files = match DataDir::from_env() {
        Ok(data) => vec![
            FooterRow {
                label: "Index",
                path: format!("{} (rebuildable)", data.index().display()),
            },
            FooterRow {
                label: "Preferences",
                path: data.prefs().display().to_string(),
            },
        ],
        Err(_) => vec![FooterRow {
            label: "Folder",
            path: "$XDG_DATA_HOME/yapkeeper (default ~/.local/share/yapkeeper)".to_string(),
        }],
    };
    let width = histories
        .iter()
        .flatten()
        .chain(&own_files)
        .map(|row| row.label.width())
        .max()
        .unwrap_or(0);
    let lines = |rows: &[FooterRow]| {
        rows.iter()
            .map(|row| format!("  {:<width$}  {}", row.label, row.path))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let histories = match &histories {
        Ok(rows) => lines(rows),
        Err(error) => format!("  {error}"),
    };
    format!(
        "Histories are read, never modified, from:\n{histories}\n\n\
         Yapkeeper's own files:\n{}",
        lines(&own_files)
    )
}

/// One line of the help footer: what a path is, then the path.
struct FooterRow {
    label: &'static str,
    path: String,
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

/// Says why the index or preferences failed and how to fix it, and gives
/// the exit code.
fn data_failure(error: DataError) -> ExitCode {
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
        Err(error) => data_failure(error),
    }
}

/// Runs `hide` or `unhide` on `id`, confirming each session it changed on
/// stderr so stdout stays empty for scripts. An id no session has is an error.
fn print_hidden(
    id: &str,
    set: fn(&Roots, &DataDir, &str) -> Result<Vec<Listed>, DataError>,
) -> ExitCode {
    let (roots, data) = match environment() {
        Ok(environment) => environment,
        Err(code) => return code,
    };
    let sessions = match set(&roots, &data, id) {
        Ok(sessions) => sessions,
        Err(error) => return data_failure(error),
    };
    if sessions.is_empty() {
        eprintln!(
            "yapkeeper: no session has the id \"{}\"",
            without_controls(id)
        );
        eprintln!("yapkeeper list --ids shows each session's id");
        return ExitCode::from(EXIT_FAILURE);
    }
    for listed in &sessions {
        let title = fit_title(&one_line(&listed.session.title));
        match listed.hidden {
            true => eprintln!("yapkeeper: hid \"{title}\"; yapkeeper search is:hidden lists it"),
            false => eprintln!("yapkeeper: unhid \"{title}\""),
        }
    }
    ExitCode::SUCCESS
}

/// The sessions `search` finds, one line each, in `list`'s format.
fn print_sessions(
    Search {
        columns,
        tools,
        query,
    }: &Search,
) -> ExitCode {
    let (roots, data) = match environment() {
        Ok(environment) => environment,
        Err(code) => return code,
    };
    let text = query.join(" ");
    let providers = roots.providers();
    let query = Query::parse(&text, &providers).including_tools(*tools);
    let listing = match search(&roots, &data, &query) {
        Ok(listing) => listing,
        Err(error) => return data_failure(error),
    };
    if listing.indexed == 0 {
        for dir in &listing.searched {
            eprintln!("yapkeeper: no sessions found in {}", dir.display());
        }
    } else if listing.sessions.is_empty()
        && query
            .tokens()
            .iter()
            .all(|token| token.kind == TokenKind::Incomplete)
    {
        // A query that asks nothing can only have lost everything to the
        // temp and hidden defaults.
        let hint = match (listing.left_out.temp, listing.left_out.hidden) {
            (true, true) => {
                "every session is hidden or in a temp folder; \
                 yapkeeper search is:temp or is:hidden lists them"
            }
            (false, true) => "every session is hidden; yapkeeper search is:hidden lists them",
            _ => "no sessions outside temp folders; yapkeeper search is:temp lists them",
        };
        eprintln!("yapkeeper: {hint}");
    } else if listing.sessions.is_empty() {
        eprintln!(
            "yapkeeper: no sessions match \"{}\"",
            without_controls(&text)
        );
        // `--tools` only finds more sessions for a word they must contain.
        let has_words = query
            .tokens()
            .iter()
            .any(|token| token.kind == TokenKind::Word && !token.negated);
        if !tools && has_words {
            eprintln!(
                "yapkeeper: --tools also searches tool calls, tool output and subagent transcripts"
            );
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
    let id_width = listing
        .sessions
        .iter()
        .map(|listed| listed.session.id.width())
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
        // First, so `cut -d' ' -f1` gives the id whatever the title holds.
        let id = match columns.ids {
            true => format!("{:<id_width$}  ", without_controls(&session.id)),
            false => String::new(),
        };
        // A closed pipe, as with `| head`, just ends the listing.
        if writeln!(
            stdout,
            "{id}{title}{padding}  {provider:<provider_width$}  {age:>3}  {state}{location}"
        )
        .is_err()
        {
            break;
        }
    }
    // After the results, so the fix is the last thing read.
    for token in query.tokens() {
        let piece = without_controls(&text[token.span.clone()]);
        match token.kind {
            TokenKind::Provider(None) => eprintln!(
                "yapkeeper: \"{piece}\" names no provider ({})",
                providers.join(", ")
            ),
            TokenKind::State(None) => eprintln!(
                "yapkeeper: \"{piece}\" names no state ({})",
                State::ALL.map(State::name).join(", ")
            ),
            _ => {}
        }
    }
    ExitCode::SUCCESS
}

/// Words rather than colour, so the state survives `NO_COLOR` and `grep`;
/// the same words `is:` takes.
fn state_label(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "",
        Availability::Orphaned => State::Orphaned.name(),
        Availability::Unavailable => State::Unavailable.name(),
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
