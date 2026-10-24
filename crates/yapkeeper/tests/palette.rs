//! Drives `yapkeeper::palette` over synthetic Claude Code, Codex and Pi histories
//! indexed into a tempdir, and checks the frames it draws into a
//! `TestBackend` and what each key press does.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;
use yapkeeper::palette::{Colour, Palette, Restore, Step};
use yapkeeper::{
    Catalog, DataDir, Indexer, REFRESH_INTERVAL, ResumeCommand, Roots, State, Stepper, hide, list,
    rebuild,
};

#[cfg(unix)]
use support::Locked;
use support::{live_dir, repo_with_worktree, repo_with_worktrees, temp_dir};

mod support;

/// A fresh `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, Pi session directory and data
/// directory, each empty.
struct Histories {
    dir: TempDir,
}

impl Histories {
    fn new() -> Histories {
        Histories {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The three sessions most tests share: two Claude Code sessions, one
    /// in a folder that exists and one whose folder is gone, and one Codex
    /// session. None is in a temp folder, so every one is listed.
    fn three() -> Histories {
        let histories = Histories::new();
        histories.claude(CLAUDE_RETRY, "/usr", "Retry the billing job");
        histories.claude(CLAUDE_GONE, "/synthetic/gone", "Release notes draft");
        histories.codex(CODEX, "/usr/bin", "Tidy the lint config");
        histories
    }

    fn roots(&self) -> Roots {
        Roots {
            claude: self.dir.path().join("claude"),
            codex: self.dir.path().join("codex"),
            pi: self.dir.path().join("pi"),
        }
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn catalog(&self) -> Catalog {
        Catalog::open(&self.roots(), &self.data()).unwrap()
    }

    /// The palette over the sessions already indexed, taking in what a
    /// stepped indexer over these histories publishes as it is stepped.
    fn palette_indexing(&self) -> (Palette, Stepper) {
        let catalog = Catalog::cached(&self.roots(), &self.data()).unwrap();
        let (indexer, stepper) = Indexer::stepped(&self.roots(), &self.data());
        let palette = Palette::new(catalog, Colour::Accent, Path::new("/")).indexing(indexer);
        (palette, stepper)
    }

    /// Writes `count` Claude Code sessions run in `/usr`, `Synthetic session 0`
    /// onwards.
    fn many(&self, count: usize) {
        for n in 0..count {
            let id = format!("aaaaaaaa-0000-4000-8000-{:012}", 1000 + n);
            self.claude(&id, "/usr", &format!("Synthetic session {n}"));
        }
    }

    /// Writes Claude Code session `id`, run in `cwd`, whose one prompt is `text`.
    fn claude(&self, id: &str, cwd: &str, text: &str) {
        let path = self
            .dir
            .path()
            .join("claude/projects/-synthetic-palette")
            .join(format!("{id}.jsonl"));
        write_records(
            &path,
            &[json!({
                "type": "user",
                "message": { "content": text },
                "cwd": cwd,
                "timestamp": "2026-10-20T10:00:00Z",
            })],
        );
    }

    /// Writes Claude Code session `id` as `records`, each given the
    /// session's `cwd` and a timestamp unless it has its own.
    fn claude_records(&self, id: &str, cwd: &str, records: &[Value]) {
        let path = self
            .dir
            .path()
            .join("claude/projects/-synthetic-palette")
            .join(format!("{id}.jsonl"));
        let records: Vec<Value> = records
            .iter()
            .map(|record| {
                let mut record = record.clone();
                record["cwd"] = json!(cwd);
                if record["timestamp"].is_null() {
                    record["timestamp"] = json!("2026-10-20T10:00:00Z");
                }
                record
            })
            .collect();
        write_records(&path, &records);
    }

    /// Writes Codex rollout `id`, run in `cwd`, whose one prompt is `text`.
    fn codex(&self, id: &str, cwd: &str, text: &str) {
        let path = self
            .dir
            .path()
            .join("codex/sessions/2026/10/22")
            .join(format!("rollout-2026-10-22T08-00-00-{id}.jsonl"));
        write_records(
            &path,
            &[
                json!({
                    "timestamp": "2026-10-22T08:00:00Z",
                    "type": "session_meta",
                    "payload": { "id": id, "cwd": cwd, "source": "cli" },
                }),
                json!({
                    "timestamp": "2026-10-22T08:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": text }],
                    },
                }),
            ],
        );
    }

    /// Writes Pi session `id`, run in `cwd`, whose one prompt is `text`.
    fn pi(&self, id: &str, cwd: &str, text: &str) {
        let path = self
            .dir
            .path()
            .join("pi/--synthetic-palette--")
            .join(format!("2026-10-22T09-00-00-000Z_{id}.jsonl"));
        write_records(
            &path,
            &[
                json!({
                    "type": "session",
                    "version": 3,
                    "id": id,
                    "timestamp": "2026-10-22T09:00:00.000Z",
                    "cwd": cwd,
                }),
                json!({
                    "type": "message",
                    "id": "p1",
                    "parentId": null,
                    "timestamp": "2026-10-22T09:00:01.000Z",
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": text }],
                    },
                }),
            ],
        );
    }
}

const CLAUDE_RETRY: &str = "aaaaaaaa-0000-4000-8000-000000000401";
const CLAUDE_GONE: &str = "aaaaaaaa-0000-4000-8000-000000000402";
const CODEX: &str = "eeeeeeee-0000-7000-8000-000000000401";
const PI: &str = "01d00000-0000-7000-8000-000000000401";

/// The Nerd Font folder icon a session in a plain folder shows.
const FOLDER_ICON: &str = "\u{f07b}";
/// The Nerd Font branch icon a session in a linked worktree shows.
const WORKTREE_ICON: &str = "\u{e725}";

fn write_records(path: &Path, records: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lines: String = records.iter().map(|record| format!("{record}\n")).collect();
    fs::write(path, lines).unwrap();
}

/// What `palette` draws into a `width`×`height` terminal.
fn draw(palette: &mut Palette, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| palette.render(frame, Instant::now()))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Each row of `buffer` as text, one character per glyph: the cell a wide
/// character spills into is skipped.
fn lines(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.top()..area.bottom())
        .map(|y| cells(buffer, y, area.left()..area.right()))
        .collect()
}

/// Row `y` of `buffer` from column `xs.start` up to `xs.end`, as `lines`
/// reads it.
fn cells(buffer: &Buffer, y: u16, xs: std::ops::Range<u16>) -> String {
    let mut line = String::new();
    let mut skip = 0;
    for x in xs {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let symbol = buffer[(x, y)].symbol();
        skip = symbol.width().saturating_sub(1);
        line.push_str(symbol);
    }
    line
}

fn frame(palette: &mut Palette, width: u16, height: u16) -> Vec<String> {
    lines(&draw(palette, width, height))
}

fn press(palette: &mut Palette, code: KeyCode) -> Step {
    palette.handle(KeyEvent::new(code, KeyModifiers::NONE), Instant::now())
}

fn ctrl(palette: &mut Palette, c: char) -> Step {
    palette.handle(
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
        Instant::now(),
    )
}

/// Types each character of `text` into the query.
fn keys(palette: &mut Palette, text: &str) {
    for c in text.chars() {
        press(palette, KeyCode::Char(c));
    }
}

/// The rows whose title starts with one of `titles`, in frame order.
fn rows_with<'f>(frame: &'f [String], titles: &[&str]) -> Vec<&'f String> {
    frame
        .iter()
        .filter(|line| {
            titles
                .iter()
                .any(|title| line.starts_with(&format!("│ {title}")))
        })
        .collect()
}

/// The results part of `row`: what lies between the left border and the
/// preview's rule, or the right border when there is no preview.
fn results(row: &str) -> &str {
    row.split('│').nth(1).unwrap_or_default()
}

/// The first word of each of the three sessions' titles, which every
/// width keeps.
const TITLES: [&str; 3] = ["Retry", "Release", "Tidy"];

/// Whether `row` ends, inside the border, with an age such as `3d`.
fn ends_with_age(row: &str) -> bool {
    let inner = row.trim_end_matches('│').trim_end();
    inner.ends_with(['m', 'h', 'd', 'w'])
        && inner[..inner.len() - 1]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit())
}

#[test]
fn a_wide_terminal_shows_the_query_line_count_rows_and_footer() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let frame = frame(&mut palette, 120, 40);
    assert!(
        frame[0].starts_with('╭') && frame[0].ends_with('╮'),
        "{frame:#?}"
    );
    assert!(
        frame[39].starts_with('╰') && frame[39].ends_with('╯'),
        "{frame:#?}"
    );
    assert!(frame[1].starts_with("│ › "), "{frame:#?}");
    assert!(
        frame[1].trim_end_matches('│').trim_end().ends_with("3 / 3"),
        "{frame:#?}"
    );
    assert!(
        frame[2].trim_matches('│').chars().all(|c| c == '─'),
        "{frame:#?}"
    );
    let rows = rows_with(&frame, &TITLES);
    assert_eq!(rows.len(), 3, "{frame:#?}");
    for row in &rows {
        assert!(results(row).contains('/'), "location: {row}");
        assert!(ends_with_age(results(row)), "age: {row}");
    }
    let usr = rows_with(&frame, &["Retry"]);
    assert!(
        usr[0].contains(&format!("{FOLDER_ICON} /usr")),
        "{}",
        usr[0]
    );
    assert!(frame[38].contains("↑↓ move"), "{frame:#?}");
}

#[test]
fn a_standard_terminal_keeps_results_in_the_left_side() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let buffer = draw(&mut palette, 80, 24);
    let frame = lines(&buffer);
    assert!(frame[1].starts_with("│ › "), "{frame:#?}");
    assert!(
        frame[1].trim_end_matches('│').trim_end().ends_with("3 / 3"),
        "{frame:#?}"
    );
    assert!(frame[22].contains("↑↓ move"), "{frame:#?}");
    // The inner width is 78; the results keep 48% of it.
    let preview_from = 1 + 78 * 48 / 100;
    assert_eq!(preview_column(&buffer), Some(preview_from), "{frame:#?}");
    let rows = rows_with(&frame, &TITLES);
    assert_eq!(rows.len(), 3, "{frame:#?}");
    for row in rows {
        assert!(ends_with_age(results(row)), "{row}");
    }
}

#[test]
fn a_narrow_terminal_gives_results_the_full_width() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let frame = frame(&mut palette, 60, 20);
    let rows = rows_with(&frame, &TITLES);
    assert_eq!(rows.len(), 3, "{frame:#?}");
    for row in rows {
        // The age ends in the last cell inside the border.
        assert_ne!(row.chars().nth(58), Some(' '), "{row}");
        assert!(ends_with_age(row), "{row}");
        assert!(row.contains('/'), "location: {row}");
    }
}

#[test]
fn typing_a_word_narrows_the_rows_and_the_count() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    keys(&mut palette, "release");
    let narrowed = frame(&mut palette, 120, 40);
    assert_eq!(rows_with(&narrowed, &TITLES).len(), 1, "{narrowed:#?}");
    assert_eq!(rows_with(&narrowed, &["Release"]).len(), 1);
    assert!(narrowed[1].contains("release"), "{narrowed:#?}");
    assert!(
        narrowed[1]
            .trim_end_matches('│')
            .trim_end()
            .ends_with("1 / 3")
    );

    for _ in "release".chars() {
        press(&mut palette, KeyCode::Backspace);
    }
    let every = frame(&mut palette, 120, 40);
    assert_eq!(rows_with(&every, &TITLES).len(), 3, "{every:#?}");
    assert!(every[1].trim_end_matches('│').trim_end().ends_with("3 / 3"));
}

#[test]
fn typing_a_filter_narrows_the_rows_and_the_count() {
    let histories = Histories::three();

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "#codex");
    let codex = frame(&mut palette, 120, 40);
    assert_eq!(rows_with(&codex, &TITLES).len(), 1, "{codex:#?}");
    assert_eq!(rows_with(&codex, &["Tidy"]).len(), 1);
    assert!(codex[1].trim_end_matches('│').trim_end().ends_with("1 / 3"));

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "retry #claude");
    let retry = frame(&mut palette, 120, 40);
    assert_eq!(rows_with(&retry, &TITLES).len(), 1, "{retry:#?}");
    assert_eq!(rows_with(&retry, &["Retry"]).len(), 1);
    assert!(retry[1].trim_end_matches('│').trim_end().ends_with("1 / 3"));
}

/// The title of the selected row: the one whose first title cell is drawn
/// bold or reversed, as only the selection bar is.
fn selected(buffer: &Buffer) -> Option<&'static str> {
    let frame = lines(buffer);
    let area = buffer.area;
    let rows: Vec<u16> = (3..area.height.saturating_sub(2))
        .filter(|&y| {
            buffer[(2, y)]
                .modifier
                .intersects(Modifier::BOLD | Modifier::REVERSED)
        })
        .collect();
    match rows.as_slice() {
        [] => None,
        [y] => TITLES
            .into_iter()
            .find(|title| frame[usize::from(*y)].starts_with(&format!("│ {title}"))),
        _ => panic!("{} rows selected: {frame:#?}", rows.len()),
    }
}

#[test]
fn the_selection_stops_at_the_first_and_last_rows() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    // Most recent first, orphaned last.
    let [first, _, last] = ["Tidy", "Retry", "Release"];

    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(first));
    press(&mut palette, KeyCode::Up);
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(first));
    for _ in 0..5 {
        assert_eq!(press(&mut palette, KeyCode::Down), Step::Continue);
    }
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(last));
    for _ in 0..5 {
        press(&mut palette, KeyCode::Up);
    }
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(first));

    for _ in 0..5 {
        assert_eq!(ctrl(&mut palette, 'n'), Step::Continue);
    }
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(last));
    for _ in 0..5 {
        ctrl(&mut palette, 'p');
    }
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some(first));
}

#[test]
fn changing_the_query_selects_the_first_row_again() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some("Release"));
    // Every title has a "t", so the rows stay the same.
    keys(&mut palette, "t");
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some("Tidy"));
}

#[test]
fn esc_clears_the_query_then_quits_and_ctrl_c_quits() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    keys(&mut palette, "release");
    assert!(frame(&mut palette, 80, 24)[22].contains("Esc clear"));
    assert_eq!(press(&mut palette, KeyCode::Esc), Step::Continue);
    let cleared = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&cleared, &TITLES).len(), 3, "{cleared:#?}");
    assert!(cleared[22].contains("Esc quit"), "{cleared:#?}");
    assert_eq!(press(&mut palette, KeyCode::Esc), Step::Quit);

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "release");
    assert_eq!(ctrl(&mut palette, 'u'), Step::Continue);
    assert_eq!(rows_with(&frame(&mut palette, 80, 24), &TITLES).len(), 3);
    keys(&mut palette, "release");
    assert_eq!(ctrl(&mut palette, 'c'), Step::Quit);
}

#[test]
fn sage_draws_the_selection_bar_grey_and_the_prompt_in_the_accent() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let buffer = draw(&mut palette, 80, 24);
    assert_eq!(buffer[(2, 1)].symbol(), "›");
    assert_eq!(buffer[(2, 1)].fg, Color::Indexed(108));
    for x in 1..30 {
        assert_eq!(buffer[(x, 3)].bg, Color::Indexed(237), "column {x}");
    }
    assert_ne!(buffer[(1, 4)].bg, Color::Indexed(237));
}

#[test]
fn mono_has_no_colour_and_a_reversed_selection() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Mono, Path::new("/"));

    let buffer = draw(&mut palette, 80, 24);
    for cell in buffer.content() {
        assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset), "{cell:?}");
    }
    let reversed: Vec<u16> = (0..24)
        .filter(|&y| buffer[(2, y)].modifier.contains(Modifier::REVERSED))
        .collect();
    assert_eq!(reversed, [3]);
    assert_eq!(selected(&buffer), Some("Tidy"));
}

#[test]
fn below_48_by_12_only_the_too_small_message_renders() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    for (width, height) in [(47, 12), (48, 11)] {
        let frame = frame(&mut palette, width, height);
        let message: Vec<usize> = (0..frame.len())
            .filter(|&y| !frame[y].trim().is_empty())
            .collect();
        assert_eq!(message.len(), 2, "{frame:#?}");
        assert_eq!(frame[message[0]].trim(), "terminal too small");
        assert_eq!(frame[message[1]].trim(), "need at least 48×12");
        assert_eq!(message[1], message[0] + 1);
    }
    assert!(frame(&mut palette, 48, 12)[0].starts_with('╭'));
}

#[test]
fn rows_name_where_a_session_ran_and_say_when_it_cannot_resume() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let frame = frame(&mut palette, 120, 40);
    let gone = rows_with(&frame, &["Release"]);
    assert!(gone[0].contains("orphaned"), "{}", gone[0]);
    let usr = rows_with(&frame, &["Retry"]);
    assert!(!usr[0].contains("orphaned"), "{}", usr[0]);

    let tmp = temp_dir();
    let (_, worktree) = repo_with_worktree(tmp.path());
    let locked = tmp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    let histories = Histories::new();
    histories.claude(CLAUDE_RETRY, worktree.to_str().unwrap(), "Worktree work");
    histories.claude(CLAUDE_GONE, locked.to_str().unwrap(), "Locked work");
    #[cfg(unix)]
    let lock = Locked::new(&locked);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "is:temp");
    let frame = self::frame(&mut palette, 120, 40);
    let worktree_row = rows_with(&frame, &["Worktree"]);
    // The location column cuts the worktree name; the project stays.
    assert!(
        worktree_row[0].contains(&format!("{WORKTREE_ICON} acme-api › bill")),
        "{frame:#?}"
    );
    #[cfg(unix)]
    if lock.denies_access() {
        let locked_row = rows_with(&frame, &["Locked"]);
        assert!(locked_row[0].contains("unavailable"), "{frame:#?}");
    }
}

#[test]
fn a_query_that_matches_only_temp_sessions_says_how_to_show_them() {
    let tmp = temp_dir();
    let histories = Histories::new();
    histories.claude(CLAUDE_RETRY, "/usr", "Retry the billing job");
    histories.claude(CLAUDE_GONE, tmp.path().to_str().unwrap(), "Scratch zebra");
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    keys(&mut palette, "zebra");
    let frame = frame(&mut palette, 80, 24);
    assert_eq!(results(&frame[3]).trim(), "No sessions match.");
    assert_eq!(results(&frame[4]).trim(), "is:temp shows temp sessions");
    assert!(frame[1].trim_end_matches('│').trim_end().ends_with("0 / 2"));
}

/// The column of the last drawn cell inside the border on row `y`.
fn last_drawn(buffer: &Buffer, y: u16) -> u16 {
    (1..buffer.area.width - 1)
        .rev()
        .find(|&x| !buffer[(x, y)].symbol().trim().is_empty())
        .unwrap()
}

#[test]
fn a_wide_title_is_cut_by_cells_and_keeps_ages_aligned() {
    let histories = Histories::new();
    let wide = "日本語のタイトルがとても長いセッションの説明です";
    histories.claude(CLAUDE_RETRY, "/usr", wide);
    histories.claude(CLAUDE_GONE, "/usr", "Retry the billing job");
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let buffer = draw(&mut palette, 60, 20);
    let frame = lines(&buffer);
    let rows: Vec<u16> = (3..5).collect();
    let wide_row = rows
        .iter()
        .find(|&&y| frame[usize::from(y)].contains("日本語"))
        .unwrap();
    assert!(frame[usize::from(*wide_row)].contains('…'), "{frame:#?}");
    assert_eq!(last_drawn(&buffer, 3), last_drawn(&buffer, 4), "{frame:#?}");
    assert_eq!(last_drawn(&buffer, 3), 58);
}

#[test]
fn the_restore_guard_leaves_the_alternate_screen_and_shows_the_cursor() {
    let leave_alternate_screen = "\x1b[?1049l";
    let show_cursor = "\x1b[?25h";

    let mut out = Vec::new();
    drop(Restore::new(&mut out));
    let written = String::from_utf8(out).unwrap();
    assert!(written.contains(leave_alternate_screen), "{written:?}");
    assert!(written.contains(show_cursor), "{written:?}");

    let mut out = Vec::new();
    let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
        let _restore = Restore::new(&mut out);
        panic!("a synthetic panic while the palette is open");
    }));
    assert!(unwound.is_err());
    let written = String::from_utf8(out).unwrap();
    assert!(written.contains(leave_alternate_screen), "{written:?}");
    assert!(written.contains(show_cursor), "{written:?}");
}

#[test]
fn the_cursor_moves_past_each_typed_space() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    keys(&mut palette, "retry");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| palette.render(frame, Instant::now()))
        .unwrap();
    let after_word = terminal.get_cursor_position().unwrap();
    keys(&mut palette, " ");
    terminal
        .draw(|frame| palette.render(frame, Instant::now()))
        .unwrap();
    let after_space = terminal.get_cursor_position().unwrap();
    assert_eq!(after_space.x, after_word.x + 1);
    assert_eq!(after_space.y, 1);
}

#[test]
fn an_empty_index_says_no_sessions_were_found() {
    let histories = Histories::new();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let frame = frame(&mut palette, 80, 24);
    assert_eq!(results(&frame[3]).trim(), "No sessions found.");
    assert!(frame[1].trim_end_matches('│').trim_end().ends_with("0 / 0"));
}

#[test]
fn a_query_that_matches_only_hidden_sessions_says_how_to_show_them() {
    let histories = Histories::three();
    hide(&histories.roots(), &histories.data(), CLAUDE_GONE).unwrap();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    assert_eq!(rows_with(&frame(&mut palette, 80, 24), &TITLES).len(), 2);
    keys(&mut palette, "release");
    let frame = frame(&mut palette, 80, 24);
    assert_eq!(results(&frame[3]).trim(), "No sessions match.");
    assert_eq!(results(&frame[4]).trim(), "is:hidden shows hidden ones");
    keys(&mut palette, " is:hidden");
    let shown = self::frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&shown, &["Release"]).len(), 1, "{shown:#?}");
}

const CLAUDE_FULL: &str = "aaaaaaaa-0000-4000-8000-000000000403";

/// Writes a Claude Code session with a title, a recap, two linked PRs, a
/// subagent and two exchanges, run in a worktree whose folder is gone.
fn full_session(histories: &Histories) {
    histories.claude_records(
        CLAUDE_FULL,
        "/synthetic/acme-api.wt/billing-retry",
        &[
            json!({ "type": "custom-title", "customTitle": "Billing retry backoff" }),
            json!({ "type": "user", "message": { "content": "Make the billing job retry with backoff" } }),
            json!({ "type": "assistant", "message": { "content": [
                { "type": "text", "text": "I added exponential backoff to the retry loop." },
                { "type": "tool_use", "name": "Bash", "input": { "command": "cargo test" } },
            ] } }),
            json!({ "type": "pr-link", "prNumber": 41 }),
            json!({ "type": "pr-link", "prNumber": 42 }),
            json!({ "type": "user", "message": { "content": "Now fix the flaky retry test" } }),
            json!({ "type": "assistant", "message": { "content": "The flaky test waited on a real clock; it now uses a fake one." } }),
            json!({ "type": "system", "subtype": "away_summary", "content": "Backoff landed in two PRs; the flaky test is fixed." }),
        ],
    );
    write_records(
        &histories
            .dir
            .path()
            .join("claude/projects/-synthetic-palette")
            .join(CLAUDE_FULL)
            .join("subagents/agent-1.jsonl"),
        &[json!({
            "type": "user",
            "isSidechain": true,
            "message": { "content": "Find every caller of the retry loop" },
            "timestamp": "2026-10-20T10:01:00Z",
        })],
    );
}

/// The column of the line between the results and the preview: the one
/// inside the border drawn as `│` on every body row, if any.
fn preview_column(buffer: &Buffer) -> Option<u16> {
    let area = buffer.area;
    (1..area.width - 1).find(|&x| (3..area.height - 2).all(|y| buffer[(x, y)].symbol() == "│"))
}

/// Each body row of the preview as text, trimmed, inside the border.
fn preview(buffer: &Buffer) -> Vec<String> {
    let column = preview_column(buffer).expect("a preview");
    let area = buffer.area;
    (3..area.height - 2)
        .map(|y| {
            cells(buffer, y, column + 1..area.width - 1)
                .trim()
                .to_string()
        })
        .collect()
}

/// The index of the first preview line that contains `text`.
fn line_with(preview: &[String], text: &str) -> usize {
    preview
        .iter()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} in {preview:#?}"))
}

#[test]
fn a_wide_terminal_previews_every_section_in_order() {
    let histories = Histories::new();
    full_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "flaky");

    let preview = preview(&draw(&mut palette, 120, 40));
    let order = [
        line_with(&preview, "Billing retry backoff"),
        line_with(&preview, "claude ·"),
        line_with(&preview, "/synthetic/acme-api.wt/billing-retry"),
        line_with(&preview, "worktree billing-retry"),
        line_with(&preview, "orphaned"),
        line_with(&preview, "Recap"),
        line_with(&preview, "Backoff landed in two PRs"),
        line_with(&preview, "Matches"),
        line_with(&preview, "Opening"),
        line_with(&preview, "Make the billing job retry"),
        line_with(&preview, "Latest"),
        preview
            .iter()
            .rposition(|line| line.contains("it now uses"))
            .unwrap(),
    ];
    assert!(order.is_sorted() && order[0] == 0, "{order:?} {preview:#?}");
    let meta = &preview[order[1]];
    for part in ["4 messages", "1 subagent", "PRs #41, #42"] {
        assert!(meta.contains(part), "{meta}");
    }
    let matches = &preview[order[7] + 1..order[8]];
    assert_eq!(
        matches.iter().filter(|line| line.contains("flaky")).count(),
        2,
        "{preview:#?}"
    );
    // Tool calls are not part of the preview.
    assert!(!preview.iter().any(|line| line.contains("cargo test")));
}

#[test]
fn matching_words_are_highlighted_in_the_preview() {
    let histories = Histories::new();
    full_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "flaky");

    let buffer = draw(&mut palette, 120, 40);
    let frame = lines(&buffer);
    let y = (3..38)
        .find(|&y| frame[y].contains("Now fix the flaky"))
        .unwrap();
    // The row holds no wide character, so each character is one cell.
    let x = frame[y][..frame[y].find("flaky").unwrap()].chars().count() as u16;
    let underlined = |x: u16| {
        buffer[(x, y as u16)]
            .modifier
            .contains(Modifier::UNDERLINED)
    };
    assert!((x..x + 5).all(underlined), "{}", frame[y]);
    assert!(!underlined(x - 1) && !underlined(x + 5), "{}", frame[y]);
    assert_eq!(buffer[(x, y as u16)].fg, Color::Indexed(108));

    let mut palette = Palette::new(histories.catalog(), Colour::Mono, Path::new("/"));
    keys(&mut palette, "flaky");
    let buffer = draw(&mut palette, 120, 40);
    assert!(
        buffer[(x, y as u16)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "{:#?}",
        lines(&buffer)
    );
}

#[test]
fn a_preview_leaves_out_what_a_session_lacks_without_gaps() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "retry");

    let preview = preview(&draw(&mut palette, 120, 40));
    let meta = &preview[line_with(&preview, "claude ·")];
    assert!(meta.contains("1 message") && !meta.contains("PR"), "{meta}");
    assert!(!meta.contains("subagent"), "{meta}");
    for absent in ["Recap", "orphaned", "worktree", "Latest"] {
        assert!(
            !preview.iter().any(|line| line.contains(absent)),
            "{absent}: {preview:#?}"
        );
    }
    let used = preview.iter().rposition(|line| !line.is_empty()).unwrap();
    let blanks: Vec<usize> = (0..=used).filter(|&y| preview[y].is_empty()).collect();
    assert_eq!(
        blanks,
        [
            line_with(&preview, "Matches") - 1,
            line_with(&preview, "Opening") - 1
        ],
        "one blank line before each section: {preview:#?}"
    );
}

#[test]
fn wide_titles_and_long_paths_stay_inside_the_preview() {
    let histories = Histories::new();
    let wide = "日本語のタイトルがとても長いセッションの説明です。".repeat(3);
    let deep = format!("/synthetic/{}end-folder", "nested-folder/".repeat(12));
    histories.claude(CLAUDE_RETRY, &deep, &wide);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let buffer = draw(&mut palette, 120, 40);
    let frame = lines(&buffer);
    for row in &frame[1..39] {
        assert!(row.ends_with('│'), "{frame:#?}");
    }
    let preview = preview(&buffer);
    // The title wraps across lines rather than being cut.
    assert!(
        preview[0].starts_with("日本語") && preview[1].contains('の'),
        "{preview:#?}"
    );
    let path = &preview[line_with(&preview, "end-folder")];
    assert!(
        path.contains('…') && path.ends_with("nested-folder/end-folder"),
        "{path}"
    );
}

#[test]
fn below_80_columns_there_is_no_preview() {
    let histories = Histories::new();
    full_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    assert!(preview_column(&draw(&mut palette, 80, 24)).is_some());
    let buffer = draw(&mut palette, 79, 24);
    assert_eq!(preview_column(&buffer), None, "{:#?}", lines(&buffer));
    assert!(!lines(&buffer).iter().any(|row| row.contains("Opening")));
}

#[test]
fn a_match_deep_in_a_long_message_shows_in_the_preview() {
    let histories = Histories::new();
    let long = format!("{} then the needle at last", "padding words ".repeat(60));
    histories.claude(CLAUDE_RETRY, "/usr", &long);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "needle");

    let buffer = draw(&mut palette, 120, 40);
    let preview = preview(&buffer);
    let matches = line_with(&preview, "Matches");
    assert!(
        preview[matches + 1].starts_with("you    …") && preview[matches + 1].contains("needle"),
        "{preview:#?}"
    );
    // The opening prompt starts from its beginning and is cut.
    let opening = line_with(&preview, "Opening");
    assert!(
        preview[opening + 1].starts_with("you    padding"),
        "{preview:#?}"
    );
    assert!(preview[opening + 3].ends_with('…'), "{preview:#?}");
}

/// The query as the query line shows it, without the prompt or the count.
fn query_text(frame: &[String]) -> &str {
    let line = frame[1].trim_start_matches("│ › ");
    line[..line.find("  ").unwrap_or(line.len())].trim_end_matches('│')
}

/// The completion list's rows, each as `(token, count)`, in order: the body
/// lines that start with a filter token and end in a number.
fn completions(frame: &[String]) -> Vec<(String, usize)> {
    frame[3..frame.len() - 2]
        .iter()
        .filter_map(|line| {
            let inner = line.trim_matches('│').trim();
            let (token, count) = inner.split_once(' ')?;
            let count = count.trim().parse().ok()?;
            token
                .starts_with(['@', '#', '!', 'i', 'n'])
                .then(|| (token.to_string(), count))
        })
        .collect()
}

fn pairs(expected: &[(&str, usize)]) -> Vec<(String, usize)> {
    expected
        .iter()
        .map(|(token, count)| (token.to_string(), *count))
        .collect()
}

#[test]
fn the_query_line_accents_filter_tokens_and_leaves_words_plain() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let query = "retry @usr !#codex not:temp is:pr !flaky #claude !orphaned tidy";
    keys(&mut palette, query);

    let buffer = draw(&mut palette, 120, 40);
    let start = 4;
    let mut at = 0;
    for token in query.split(' ') {
        let filter = !matches!(token, "retry" | "!flaky" | "tidy");
        for x in at..at + token.len() {
            let cell = &buffer[(start + x as u16, 1)];
            match filter {
                true => assert_eq!(cell.fg, Color::Indexed(108), "{token} at {x}"),
                false => assert_eq!(cell.fg, Color::Reset, "{token} at {x}"),
            }
        }
        at += token.len() + 1;
    }
}

#[test]
fn each_filter_prefix_opens_its_completions_with_session_counts() {
    let histories = Histories::three();
    let cases: [(&str, &[(&str, usize)]); 4] = [
        ("#", &[("#claude", 2), ("#codex", 1), ("#pi", 0)]),
        (
            "is:",
            &[
                ("is:orphaned", 1),
                ("is:temp", 0),
                ("is:git", 0),
                ("is:folder", 3),
                ("is:pr", 0),
                ("is:recap", 0),
                ("is:unavailable", 0),
                ("is:hidden", 0),
            ],
        ),
        ("!c", &[("!claude", 1), ("!codex", 2)]),
        ("not:o", &[("not:orphaned", 2)]),
    ];
    for (typed, expected) in cases {
        let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
        keys(&mut palette, typed);
        let frame = frame(&mut palette, 120, 40);
        assert_eq!(completions(&frame), pairs(expected), "{typed}: {frame:#?}");
    }

    // Most recent first, the missing folder left out; `@/usr` also finds
    // `/usr/bin`.
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@");
    let frame = frame(&mut palette, 120, 40);
    let rows = location_rows(&frame);
    let rows: Vec<(&str, usize)> = rows
        .iter()
        .map(|row| (row.label.as_str(), row.count))
        .collect();
    assert_eq!(rows, [("/usr/bin", 1), ("/usr", 2)], "{frame:#?}");
    assert!(
        !frame.iter().any(|line| line.contains("gone")),
        "{frame:#?}"
    );
}

#[test]
fn completions_name_every_provider_and_state_the_library_knows() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "!");
    let listed: Vec<String> = completions(&frame(&mut palette, 120, 40))
        .into_iter()
        .map(|(token, _)| token)
        .collect();
    let named: Vec<String> = histories
        .roots()
        .providers()
        .into_iter()
        .chain(State::ALL.map(State::name))
        .map(|name| format!("!{name}"))
        .collect();
    assert_eq!(listed, named);
}

#[test]
fn typing_after_the_prefix_narrows_the_completions() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "!is:o");
    assert_eq!(
        completions(&frame(&mut palette, 120, 40)),
        pairs(&[("!is:orphaned", 2)])
    );
    keys(&mut palette, "x");
    assert_eq!(completions(&frame(&mut palette, 120, 40)), []);

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@bin");
    let rows = location_rows(&frame(&mut palette, 120, 40));
    let rows: Vec<(&str, usize)> = rows
        .iter()
        .map(|row| (row.label.as_str(), row.count))
        .collect();
    assert_eq!(rows, [("/usr/bin", 1)]);
}

#[test]
fn tab_or_enter_inserts_the_selected_completion() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "tidy #c");
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Tab);
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "tidy #codex", "{frame:#?}");
    assert_eq!(completions(&frame), []);
    assert_eq!(rows_with(&frame, &TITLES).len(), 1, "{frame:#?}");
    assert!(frame[1].trim_end_matches('│').trim_end().ends_with("1 / 3"));

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "NOT:");
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Up);
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);
    let frame = self::frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "NOT:claude", "{frame:#?}");
    assert_eq!(rows_with(&frame, &["Tidy"]).len(), 1, "{frame:#?}");
}

#[test]
fn a_space_never_opens_completions() {
    let histories = Histories::three();
    for typed in ["is", "is ", "is pr", "#claude ", "! "] {
        let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
        keys(&mut palette, typed);
        let frame = frame(&mut palette, 120, 40);
        assert_eq!(completions(&frame), [], "{typed}: {frame:#?}");
    }
}

#[test]
fn ctrl_l_inserts_where_the_palette_was_launched() {
    let histories = Histories::three();
    let tmp = temp_dir();
    let (repo, worktree) = repo_with_worktree(tmp.path());
    let launched = [
        (repo.join("src"), "@acme-api".to_string()),
        (worktree, "@acme-api:billing-retry".to_string()),
        (PathBuf::from("/usr/bin"), "retry @/usr/bin".to_string()),
    ];
    for (dir, expected) in launched {
        let mut palette = Palette::new(histories.catalog(), Colour::Accent, &dir);
        if dir.starts_with("/usr") {
            keys(&mut palette, "retry ");
        }
        assert_eq!(ctrl(&mut palette, 'l'), Step::Continue);
        let frame = frame(&mut palette, 120, 40);
        assert_eq!(query_text(&frame), expected, "{frame:#?}");
        assert_eq!(completions(&frame), [], "{frame:#?}");
    }
}

#[test]
fn a_filter_typed_out_in_full_shows_its_sessions() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "#CODEX");
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(completions(&frame), [], "{frame:#?}");
    assert_eq!(rows_with(&frame, &["Tidy"]).len(), 1, "{frame:#?}");
}

#[test]
fn a_launch_folder_that_is_gone_offers_no_ctrl_l() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new(""));
    assert!(!frame(&mut palette, 120, 40)[38].contains("^L"));
    ctrl(&mut palette, 'l');
    assert!(query_text(&frame(&mut palette, 120, 40)).starts_with("search everything"));

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert!(frame(&mut palette, 120, 40)[38].contains("^L here"));
}

#[test]
fn a_query_cut_from_the_left_keeps_its_filters_accented() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let query = format!("{} #claude", "retry ".repeat(20));
    keys(&mut palette, &query);

    let buffer = draw(&mut palette, 60, 20);
    let frame = lines(&buffer);
    assert!(frame[1].contains("│ › …"), "{frame:#?}");
    let end = frame[1].find("#claude").unwrap();
    let x = frame[1][..end].width() as u16;
    for at in x..x + 7 {
        assert_eq!(buffer[(at, 1)].fg, Color::Indexed(108), "{frame:#?}");
    }
    assert_eq!(buffer[(x - 2, 1)].fg, Color::Reset, "{frame:#?}");
}

/// One row of the location list as drawn.
#[derive(Debug, PartialEq, Eq)]
struct LocationLine {
    /// Drawn under its project's row.
    indented: bool,
    label: String,
    detail: String,
    count: usize,
}

/// The location list's rows, in order: each body line between the borders
/// that reads as an icon, a label, a detail and a count, apart by runs of
/// two or more spaces.
fn location_rows(frame: &[String]) -> Vec<LocationLine> {
    frame[3..frame.len() - 2]
        .iter()
        .filter_map(|line| {
            let inner = line.strip_prefix('│')?.strip_suffix('│')?;
            let indented = inner.starts_with("    ");
            let mut chars = inner.trim_start().chars();
            chars.next()?;
            let rest = chars.as_str().trim();
            let parts: Vec<&str> = rest
                .split("  ")
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect();
            let [label, detail, count] = parts.as_slice() else {
                return None;
            };
            Some(LocationLine {
                indented,
                label: label.to_string(),
                detail: detail.to_string(),
                count: count.parse().ok()?,
            })
        })
        .collect()
}

/// Fixture F: `acme-api` with worktrees `billing-retry` and `docs-site`, and
/// `lint-kit` with a main checkout only, under `<live dir>/code`, one session
/// in each checkout and worktree, one in `/usr` and one in `/synthetic/gone`.
/// `lint-kit` is the most recent repo, and `/usr` is newer than either.
fn locations_fixture() -> (Histories, TempDir) {
    let live = live_dir();
    let code = live.path().join("code");
    let (acme, acme_worktrees) =
        repo_with_worktrees(&code, "acme-api", &["billing-retry", "docs-site"]);
    let (lint, _) = repo_with_worktrees(&code, "lint-kit", &[]);
    let histories = Histories::new();
    let sessions = [
        ("501", "/synthetic/gone".into(), "Gone notes", "2026-10-10"),
        ("502", acme, "Acme main work", "2026-10-11"),
        (
            "503",
            acme_worktrees[1].clone(),
            "Docs site copy",
            "2026-10-12",
        ),
        (
            "504",
            acme_worktrees[0].clone(),
            "Billing retry fix",
            "2026-10-13",
        ),
        ("505", lint, "Lint kit rules", "2026-10-15"),
        (
            "506",
            PathBuf::from("/usr"),
            "Usr folder chores",
            "2026-10-18",
        ),
    ];
    for (id, cwd, title, day) in sessions {
        histories.claude_records(
            &format!("aaaaaaaa-0000-4000-8000-000000000{id}"),
            &cwd.to_string_lossy(),
            &[json!({
                "type": "user",
                "message": { "content": title },
                "timestamp": format!("{day}T10:00:00Z"),
            })],
        );
    }
    (histories, live)
}

/// What a test expects of one location row: its detail starts with `kind`
/// and ends with `path`, the path's last folders, which hold no home folder.
struct Expected {
    indented: bool,
    label: &'static str,
    kind: &'static str,
    path: &'static str,
    count: usize,
}

/// Checks fixture F's rows with nothing typed after `@`, in the documented
/// order.
fn assert_documented_order(rows: &[LocationLine]) {
    let expected = [
        Expected {
            indented: false,
            label: "lint-kit",
            kind: "git repo · ",
            path: "/code/lint-kit",
            count: 1,
        },
        Expected {
            indented: false,
            label: "acme-api",
            kind: "git repo · 2 worktrees · ",
            path: "/code/acme-api",
            count: 3,
        },
        Expected {
            indented: true,
            label: "billing-retry",
            kind: "worktree · ",
            path: "/acme-api.wt/billing-retry",
            count: 1,
        },
        Expected {
            indented: true,
            label: "docs-site",
            kind: "worktree · ",
            path: "/acme-api.wt/docs-site",
            count: 1,
        },
        Expected {
            indented: false,
            label: "/usr",
            kind: "folder",
            path: "folder",
            count: 1,
        },
    ];
    assert_eq!(rows.len(), expected.len(), "{rows:#?}");
    for (row, want) in rows.iter().zip(expected) {
        assert_eq!(
            (row.indented, row.label.as_str(), row.count),
            (want.indented, want.label, want.count),
            "{rows:#?}"
        );
        assert!(
            row.detail.starts_with(want.kind) && row.detail.ends_with(want.path),
            "{row:?}"
        );
    }
}

#[test]
fn tab_opens_the_location_list_in_the_documented_order() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "@", "{frame:#?}");
    assert_documented_order(&location_rows(&frame));
    assert!(
        !frame.iter().any(|line| line.contains("gone")),
        "{frame:#?}"
    );
}

#[test]
fn a_missing_folder_shows_last_once_anything_is_typed() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@/");
    let mut rows = location_rows(&frame(&mut palette, 120, 40));
    let last = rows.pop();
    assert_documented_order(&rows);
    assert_eq!(
        last,
        Some(LocationLine {
            indented: false,
            label: "/synthetic/gone".to_string(),
            detail: "missing · folder no longer exists".to_string(),
            count: 1,
        })
    );
}

#[test]
fn typing_at_or_pressing_tab_opens_the_same_location_list() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@");
    assert_documented_order(&location_rows(&frame(&mut palette, 120, 40)));

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "retry");
    press(&mut palette, KeyCode::Tab);
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "retry @", "{frame:#?}");
    assert_documented_order(&location_rows(&frame));

    // With no locations to choose, a second `Tab` types no second `@`.
    let empty = Histories::new();
    let mut palette = Palette::new(empty.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    press(&mut palette, KeyCode::Tab);
    assert_eq!(query_text(&self::frame(&mut palette, 120, 40)), "@");
}

#[test]
fn missing_folders_are_hidden_until_typed() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@");
    let rows = location_rows(&frame(&mut palette, 120, 40));
    assert!(
        !rows.iter().any(|row| row.label.contains("gone")),
        "{rows:#?}"
    );

    keys(&mut palette, "gone");
    assert_eq!(
        location_rows(&frame(&mut palette, 120, 40)),
        [LocationLine {
            indented: false,
            label: "/synthetic/gone".to_string(),
            detail: "missing · folder no longer exists".to_string(),
            count: 1,
        }]
    );
}

#[test]
fn choosing_a_worktree_inserts_its_token_and_narrows_the_results() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "@acme-api:billing-retry", "{frame:#?}");
    assert_eq!(location_rows(&frame), [], "{frame:#?}");
    let titles = [
        "Gone notes",
        "Acme main work",
        "Docs site copy",
        "Billing retry fix",
        "Lint kit rules",
        "Usr folder chores",
    ];
    let shown: Vec<&String> = rows_with(&frame, &titles);
    assert_eq!(shown.len(), 1, "{frame:#?}");
    assert!(shown[0].starts_with("│ Billing retry fix"), "{frame:#?}");
    assert!(frame[1].trim_end_matches('│').trim_end().ends_with("1 / 6"));
}

#[test]
fn choosing_a_project_or_a_folder_inserts_its_token() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    press(&mut palette, KeyCode::Enter);
    assert_eq!(query_text(&frame(&mut palette, 120, 40)), "@lint-kit");

    ctrl(&mut palette, 'u');
    press(&mut palette, KeyCode::Tab);
    for _ in 0..4 {
        press(&mut palette, KeyCode::Down);
    }
    press(&mut palette, KeyCode::Tab);
    assert_eq!(query_text(&frame(&mut palette, 120, 40)), "@/usr");
}

#[test]
fn typing_after_at_matches_any_part_of_a_location_path() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@code");
    let rows = location_rows(&frame(&mut palette, 120, 40));
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        labels,
        ["lint-kit", "acme-api", "billing-retry", "docs-site"],
        "{rows:#?}"
    );

    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "@bin");
    assert_eq!(
        location_rows(&frame(&mut palette, 120, 40)),
        [LocationLine {
            indented: false,
            label: "/usr/bin".to_string(),
            detail: "folder".to_string(),
            count: 1,
        }]
    );
}

#[test]
fn location_rows_keep_their_kind_words_and_counts_in_narrow_terminals() {
    let (histories, _live) = locations_fixture();
    // At 50 columns `acme-api`'s path would have room for only its `…`.
    for (width, height) in [(80, 24), (60, 20), (50, 12), (48, 12)] {
        let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
        keys(&mut palette, "@/");
        let buffer = draw(&mut palette, width, height);
        let frame = lines(&buffer);
        let rows = location_rows(&frame);
        assert_eq!(rows.len(), 6, "{width}×{height}: {frame:#?}");
        for row in &rows {
            assert!(
                ["git repo", "worktree", "folder", "missing"]
                    .iter()
                    .any(|kind| row.detail.starts_with(kind)),
                "{width}×{height}: {row:?}"
            );
            assert!(!row.detail.ends_with("· …"), "{width}×{height}: {row:?}");
        }
        for y in 3..3 + rows.len() as u16 {
            assert_eq!(buffer[(0, y)].symbol(), "│", "{width}×{height}: {frame:#?}");
            assert_eq!(buffer[(width - 1, y)].symbol(), "│", "{frame:#?}");
            let last = buffer[(width - 2, y)].symbol();
            assert!(
                last.chars().all(|c| c.is_ascii_digit()) && !last.is_empty(),
                "{width}×{height} row {y}: {frame:#?}"
            );
        }
    }
}

#[test]
fn mono_reverses_only_the_selected_location_row() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Mono, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    press(&mut palette, KeyCode::Down);
    let buffer = draw(&mut palette, 80, 24);
    for cell in buffer.content() {
        assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset), "{cell:?}");
    }
    let rows = location_rows(&lines(&buffer)).len() as u16;
    for y in 3..3 + rows {
        for x in 1..79 {
            let reversed = buffer[(x, y)].modifier.contains(Modifier::REVERSED);
            assert_eq!(reversed, y == 4, "({x}, {y})");
        }
    }
}

#[test]
fn the_footer_names_tab_for_locations_until_the_list_is_open() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let footer = frame(&mut palette, 120, 40)[38].clone();
    assert!(footer.contains("Tab locations"), "{footer}");
    press(&mut palette, KeyCode::Tab);
    let footer = frame(&mut palette, 120, 40)[38].clone();
    assert!(footer.contains("Tab insert"), "{footer}");
    assert!(!footer.contains("Tab locations"), "{footer}");
}

/// The rows of the conversation's body, inside the border, trimmed: from
/// under the rule below its title and details down to the footer.
fn conversation_body(frame: &[String]) -> Vec<String> {
    frame[4..frame.len() - 2]
        .iter()
        .map(|row| row.trim_matches('│').trim().to_string())
        .collect()
}

/// Writes a Claude Code session of forty exchanges, `turn 0` to `turn 39`,
/// whose prompts 5, 20 and 35 mention a needle, and whose first reply
/// calls a tool with a long command.
fn long_session(histories: &Histories) {
    let mut records = Vec::new();
    for turn in 0..40 {
        let prompt = match turn {
            5 | 20 | 35 => format!("turn {turn} finds the needle"),
            _ => format!("turn {turn} asks"),
        };
        records.push(json!({ "type": "user", "message": { "content": prompt } }));
        let mut reply = vec![json!({ "type": "text", "text": format!("reply {turn}") })];
        if turn == 0 {
            let command = "cargo test --workspace ".repeat(10);
            reply.push(
                json!({ "type": "tool_use", "name": "Bash", "input": { "command": command } }),
            );
        }
        records.push(json!({ "type": "assistant", "message": { "content": reply } }));
    }
    histories.claude_records(CLAUDE_FULL, "/usr", &records);
}

#[test]
fn right_opens_the_conversation_and_esc_or_left_returns_unchanged() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    // Every title has a "t", so all three rows stay.
    keys(&mut palette, "t");
    press(&mut palette, KeyCode::Down);
    let palette_frame = frame(&mut palette, 80, 24);
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some("Retry"));

    for back in [KeyCode::Esc, KeyCode::Left] {
        assert_eq!(press(&mut palette, KeyCode::Right), Step::Continue);
        let reading = frame(&mut palette, 80, 24);
        assert!(reading[1].contains("Retry the billing job"), "{reading:#?}");
        let body = conversation_body(&reading);
        assert!(body[0].starts_with("you"), "{reading:#?}");
        assert!(body[0].contains("Retry the billing job"), "{reading:#?}");
        assert!(reading[22].contains("Esc back"), "{reading:#?}");

        assert_eq!(press(&mut palette, back), Step::Continue);
        assert_eq!(frame(&mut palette, 80, 24), palette_frame);
    }
}

#[test]
fn printable_keys_leave_the_query_alone_while_reading() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let palette_frame = frame(&mut palette, 80, 24);

    press(&mut palette, KeyCode::Right);
    let reading = frame(&mut palette, 80, 24);
    keys(&mut palette, "xyz");
    assert_eq!(frame(&mut palette, 80, 24), reading);
    press(&mut palette, KeyCode::Esc);
    assert_eq!(frame(&mut palette, 80, 24), palette_frame);

    press(&mut palette, KeyCode::Right);
    assert_eq!(ctrl(&mut palette, 'c'), Step::Quit);
}

#[test]
fn arrows_and_page_keys_scroll_the_conversation() {
    let histories = Histories::new();
    long_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Right);
    let first_row = |palette: &mut Palette| conversation_body(&frame(palette, 80, 24))[0].clone();

    assert!(first_row(&mut palette).contains("turn 0 asks"));
    press(&mut palette, KeyCode::Up);
    assert!(first_row(&mut palette).contains("turn 0 asks"));
    // A blank line sits between messages.
    press(&mut palette, KeyCode::Down);
    assert_eq!(first_row(&mut palette), "");
    press(&mut palette, KeyCode::Down);
    assert!(first_row(&mut palette).contains("reply 0"));

    // A page is the body's height: 24 rows less the border, the title,
    // the details, the rule and the footer.
    press(&mut palette, KeyCode::PageDown);
    let paged = conversation_body(&frame(&mut palette, 80, 24));
    assert_eq!(paged.len(), 18);
    press(&mut palette, KeyCode::PageUp);
    assert!(first_row(&mut palette).contains("reply 0"));

    press(&mut palette, KeyCode::End);
    let end = conversation_body(&frame(&mut palette, 80, 24));
    assert!(end.last().unwrap().contains("reply 39"), "{end:#?}");
    // The end stays put.
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::PageDown);
    assert_eq!(conversation_body(&frame(&mut palette, 80, 24)), end);
    press(&mut palette, KeyCode::Home);
    assert!(first_row(&mut palette).contains("turn 0 asks"));
}

#[test]
fn a_page_down_moves_the_last_row_to_the_top() {
    let histories = Histories::new();
    long_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Right);

    let before = conversation_body(&frame(&mut palette, 80, 24));
    press(&mut palette, KeyCode::PageDown);
    let after = conversation_body(&frame(&mut palette, 80, 24));
    assert_eq!(after[0], before[before.len() - 1], "{before:#?} {after:#?}");
}

/// Which `turn N finds the needle` prompts `body` shows.
fn needles(body: &[String]) -> Vec<String> {
    body.iter()
        .filter(|row| row.contains("needle"))
        .map(|row| row.split_whitespace().nth(2).unwrap().to_string())
        .collect()
}

#[test]
fn n_and_shift_n_move_between_matches_and_wrap_around() {
    let histories = Histories::new();
    long_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "needle");
    press(&mut palette, KeyCode::Right);

    // Opening shows the first match; each one is the only match a page holds.
    let reading = frame(&mut palette, 80, 24);
    assert_eq!(needles(&conversation_body(&reading)), ["5"]);
    assert!(reading[1].contains("match 1 / 3"), "{reading:#?}");
    let mut seen = Vec::new();
    for key in ['n', 'n', 'n', 'N', 'N'] {
        press(&mut palette, KeyCode::Char(key));
        let reading = frame(&mut palette, 80, 24);
        let header = reading[1]
            .split("match ")
            .nth(1)
            .unwrap()
            .trim_end_matches(['│', ' ']);
        seen.push((needles(&conversation_body(&reading)), header.to_string()));
    }
    let expected = [
        ("20", "2 / 3"),
        ("35", "3 / 3"),
        ("5", "1 / 3"),
        ("35", "3 / 3"),
        ("20", "2 / 3"),
    ];
    let expected: Vec<(Vec<String>, String)> = expected
        .iter()
        .map(|(turn, header)| (vec![turn.to_string()], header.to_string()))
        .collect();
    assert_eq!(seen, expected);
}

#[test]
fn matches_are_highlighted_in_the_conversation() {
    let histories = Histories::new();
    long_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Mono, Path::new("/"));
    keys(&mut palette, "needle");
    press(&mut palette, KeyCode::Right);

    let buffer = draw(&mut palette, 80, 24);
    let frame = lines(&buffer);
    let y = frame
        .iter()
        .position(|row| row.contains("finds the needle"))
        .unwrap();
    let x = frame[y][..frame[y].find("needle").unwrap()].chars().count() as u16;
    let underlined = |x: u16| {
        buffer[(x, y as u16)]
            .modifier
            .contains(Modifier::UNDERLINED)
    };
    assert!((x..x + 6).all(underlined), "{}", frame[y]);
    assert!(!underlined(x - 1) && !underlined(x + 6), "{}", frame[y]);
}

#[test]
fn a_tool_call_reads_as_one_line() {
    let histories = Histories::new();
    long_session(&histories);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Right);

    let body = conversation_body(&frame(&mut palette, 80, 24));
    let call = body.iter().position(|row| row.contains("Bash")).unwrap();
    assert!(body[call].ends_with('…'), "{body:#?}");
    assert!(body[call + 1].is_empty(), "{body:#?}");
}

#[test]
fn at_60_columns_right_opens_the_conversation_inside_the_border() {
    let histories = Histories::new();
    let long = format!("{} then the needle at last", "padding words ".repeat(20));
    histories.claude(CLAUDE_RETRY, "/usr", &long);
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "needle");
    press(&mut palette, KeyCode::Right);

    let reading = frame(&mut palette, 60, 20);
    for row in &reading[1..19] {
        assert!(row.starts_with('│') && row.ends_with('│'), "{reading:#?}");
        assert_eq!(row.width(), 60, "{reading:#?}");
    }
    // The long prompt wraps, whole, rather than being cut.
    let body = conversation_body(&reading).join(" ");
    assert!(body.contains("then the needle at last"), "{reading:#?}");
}

#[test]
fn ctrl_x_hides_the_selected_session_for_good_and_is_hidden_brings_it_back() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert_eq!(selected(&draw(&mut palette, 80, 24)), Some("Tidy"));
    let hints = frame(&mut palette, 120, 24);
    assert!(hints[22].contains("^X hide  ^T tools"), "{hints:#?}");

    ctrl(&mut palette, 'x');
    let hid = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&hid, &TITLES).len(), 2, "{hid:#?}");
    assert!(rows_with(&hid, &["Tidy"]).is_empty(), "{hid:#?}");
    assert!(hid[22].contains("Hid “Tidy the lint config”"), "{hid:#?}");
    assert!(hid[22].contains("is:hidden"), "{hid:#?}");

    let mut relaunched = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert!(rows_with(&frame(&mut relaunched, 80, 24), &["Tidy"]).is_empty());
    keys(&mut relaunched, "is:hidden ");
    let hidden = frame(&mut relaunched, 120, 24);
    assert!(hidden[22].contains("^X unhide  ^T tools"), "{hidden:#?}");
    assert_eq!(rows_with(&hidden, &TITLES).len(), 1, "{hidden:#?}");
    assert_eq!(rows_with(&hidden, &["Tidy"]).len(), 1, "{hidden:#?}");
    ctrl(&mut relaunched, 'x');
    let unhid = frame(&mut relaunched, 80, 24);
    assert!(rows_with(&unhid, &TITLES).is_empty(), "{unhid:#?}");
    assert!(
        unhid[22].contains("Unhid “Tidy the lint config”"),
        "{unhid:#?}"
    );
    ctrl(&mut relaunched, 'u');
    assert_eq!(rows_with(&frame(&mut relaunched, 80, 24), &TITLES).len(), 3);

    let mut again = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert_eq!(rows_with(&frame(&mut again, 80, 24), &TITLES).len(), 3);
}

#[test]
fn the_toast_gives_way_to_the_key_hints_after_three_seconds() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let footer = |palette: &mut Palette, at: Instant| {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| palette.render(frame, at)).unwrap();
        lines(terminal.backend().buffer())[22].clone()
    };
    assert_eq!(palette.redraw_at(), None);

    let pressed = Instant::now();
    let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
    palette.handle(ctrl_x, pressed);
    let gone = pressed + Duration::from_secs(3);
    assert_eq!(palette.redraw_at(), Some(gone));
    let shown = footer(&mut palette, gone - Duration::from_millis(1));
    assert!(shown.contains("is:hidden"), "{shown}");

    let hints = footer(&mut palette, gone);
    assert!(hints.contains("Esc quit"), "{hints}");
    assert_eq!(palette.redraw_at(), None);
}

#[test]
fn ctrl_t_searches_tool_calls_too_and_says_so_on_the_query_line() {
    let histories = Histories::three();
    histories.claude_records(
        CLAUDE_FULL,
        "/usr",
        &[
            json!({ "type": "user", "message": { "content": "Run the suite" } }),
            json!({ "type": "assistant", "message": { "content": [
                { "type": "tool_use", "name": "Bash", "input": { "command": "cargo nextest run" } },
            ] } }),
        ],
    );
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    keys(&mut palette, "nextest");
    let messages_only = frame(&mut palette, 80, 24);
    assert_eq!(results(&messages_only[3]).trim(), "No sessions match.");
    assert_eq!(
        results(&messages_only[4]).trim(),
        "^T searches tool calls too"
    );
    assert!(!messages_only[1].contains("+tools"), "{messages_only:#?}");

    ctrl(&mut palette, 't');
    let buffer = draw(&mut palette, 80, 24);
    let with_tools = lines(&buffer);
    assert!(
        with_tools[1].ends_with("1 / 4 · +tools│"),
        "{with_tools:#?}"
    );
    assert_eq!(rows_with(&with_tools, &["Run the suite"]).len(), 1);
    let x = with_tools[1].chars().count() as u16 - 7;
    assert_eq!(cells(&buffer, 1, x..x + 6), "+tools");
    assert_eq!(buffer[(x, 1)].fg, Color::Indexed(108));
    assert!(
        with_tools[22].contains("Searching tool calls too"),
        "{with_tools:#?}"
    );

    ctrl(&mut palette, 't');
    let again = frame(&mut palette, 80, 24);
    assert_eq!(results(&again[3]).trim(), "No sessions match.");
    assert!(again[22].contains("Searching messages only"), "{again:#?}");
}

/// The settings screen's labels, in order.
const SETTINGS: [&str; 5] = [
    "Hide temp-folder sessions",
    "Hide missing folders in locations",
    "List orphans last",
    "Accent",
    "Location icons",
];

/// The value the settings screen shows on the row labelled `label`: what
/// follows the label's run of spaces, inside the border.
fn setting(frame: &[String], label: &str) -> String {
    let row = frame
        .iter()
        .find(|row| row.trim_start_matches('│').trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("no {label} row: {frame:#?}"));
    let inner = row.trim_matches('│');
    inner.trim()[label.len()..].trim().to_string()
}

/// Whether the settings screen's line labelled `label` shows `path`, whole
/// or cut from the left with `…`.
fn shows_path(frame: &[String], label: &str, path: &Path) -> bool {
    let path = path.to_string_lossy();
    frame.iter().any(|row| {
        let Some(rest) = row.trim_matches('│').trim().strip_prefix(label) else {
            return false;
        };
        let shown = rest.trim();
        shown == path
            || shown
                .strip_prefix('…')
                .is_some_and(|end| path.ends_with(end))
    })
}

/// `row` with every run of spaces made one.
fn words(row: &str) -> String {
    row.trim_matches('│')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn ctrl_s_opens_settings_with_the_defaults_and_where_the_files_live() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    assert_eq!(ctrl(&mut palette, 's'), Step::Continue);
    let frame = frame(&mut palette, 80, 24);
    assert_eq!(frame[1].trim_matches('│').trim(), "Settings", "{frame:#?}");
    let values: Vec<String> = SETTINGS
        .iter()
        .map(|label| setting(&frame, label))
        .collect();
    assert_eq!(values[..4], ["on", "on", "on", "Sage"], "{frame:#?}");
    assert!(values[4].starts_with("Nerd Font"), "{frame:#?}");
    let data = histories.data();
    assert!(shows_path(&frame, "index", &data.index()), "{frame:#?}");
    assert!(
        shows_path(&frame, "preferences", &data.prefs()),
        "{frame:#?}"
    );
    assert_eq!(words(&frame[22]), "↑↓ move ←→ change Esc back ^C quit");
}

/// The label of the setting the selection bar is on.
fn selected_setting(buffer: &Buffer) -> Option<&'static str> {
    let frame = lines(buffer);
    let rows: Vec<usize> = (3..frame.len() - 2)
        .filter(|&y| buffer[(2, y as u16)].bg == Color::Indexed(237))
        .collect();
    match rows.as_slice() {
        [] => None,
        [y] => SETTINGS.into_iter().find(|label| {
            frame[*y]
                .trim_start_matches('│')
                .trim_start()
                .starts_with(label)
        }),
        _ => panic!("{} rows selected: {frame:#?}", rows.len()),
    }
}

#[test]
fn settings_keep_the_palette_as_it_was_and_swallow_printable_keys() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    // Every title has a "t", so all three rows stay.
    keys(&mut palette, "t");
    press(&mut palette, KeyCode::Down);
    let palette_frame = frame(&mut palette, 80, 24);

    for back in [KeyCode::Esc, KeyCode::Char('s')] {
        ctrl(&mut palette, 's');
        assert_eq!(
            frame(&mut palette, 80, 24)[1].trim_matches('│').trim(),
            "Settings"
        );
        let step = match back {
            KeyCode::Char(c) => ctrl(&mut palette, c),
            code => press(&mut palette, code),
        };
        assert_eq!(step, Step::Continue);
        assert_eq!(frame(&mut palette, 80, 24), palette_frame);
    }

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    ctrl(&mut palette, 's');
    keys(&mut palette, "abc");
    press(&mut palette, KeyCode::Esc);
    // An empty query shows the placeholder.
    let query = frame(&mut palette, 80, 24);
    assert!(
        query_text(&query).starts_with("search everything"),
        "{query:#?}"
    );

    ctrl(&mut palette, 's');
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[0])
    );
    press(&mut palette, KeyCode::Up);
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[0])
    );
    press(&mut palette, KeyCode::Down);
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[1])
    );
    for _ in 0..6 {
        ctrl(&mut palette, 'n');
    }
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[4])
    );
    press(&mut palette, KeyCode::Down);
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[4])
    );
    ctrl(&mut palette, 'p');
    assert_eq!(
        selected_setting(&draw(&mut palette, 80, 24)),
        Some(SETTINGS[3])
    );
    assert_eq!(ctrl(&mut palette, 'c'), Step::Quit);
}

#[test]
fn the_footer_names_ctrl_s_for_settings() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let footer = frame(&mut palette, 120, 24)[22].clone();
    assert!(
        footer.contains("^T tools  ^S settings  ^C quit"),
        "{footer}"
    );
}

/// Opens settings, moves to the setting labelled `label`, presses `change`
/// on it and closes settings again.
fn change_setting(palette: &mut Palette, label: &str, change: KeyCode) {
    let row = SETTINGS
        .iter()
        .position(|setting| *setting == label)
        .unwrap();
    ctrl(palette, 's');
    for _ in 0..row {
        press(palette, KeyCode::Down);
    }
    press(palette, change);
    press(palette, KeyCode::Esc);
}

#[test]
fn hide_temp_sessions_leaves_them_out_and_the_header_counts_them() {
    let tmp = temp_dir();
    let histories = Histories::three();
    let scratch = tmp.path().to_str().unwrap();
    histories.claude(
        "aaaaaaaa-0000-4000-8000-000000000403",
        scratch,
        "Scratch one",
    );
    histories.claude(
        "aaaaaaaa-0000-4000-8000-000000000404",
        scratch,
        "Scratch two",
    );
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));

    let on = frame(&mut palette, 120, 24);
    assert!(on[1].ends_with("2 temp hidden · 3 / 5│"), "{on:#?}");
    assert!(rows_with(&on, &["Scratch"]).is_empty(), "{on:#?}");

    change_setting(&mut palette, SETTINGS[0], KeyCode::Right);
    let off = frame(&mut palette, 120, 24);
    assert_eq!(rows_with(&off, &["Scratch"]).len(), 2, "{off:#?}");
    assert!(off[1].ends_with(" 5 / 5│"), "{off:#?}");
    assert!(!off[1].contains("temp hidden"), "{off:#?}");

    change_setting(&mut palette, SETTINGS[0], KeyCode::Right);
    keys(&mut palette, "is:temp ");
    let asked = frame(&mut palette, 120, 24);
    assert_eq!(rows_with(&asked, &["Scratch"]).len(), 2, "{asked:#?}");
    assert!(!asked[1].contains("temp hidden"), "{asked:#?}");
}

#[test]
fn hide_missing_folders_off_lists_them_last_in_the_open_location_list() {
    let (histories, _live) = locations_fixture();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    press(&mut palette, KeyCode::Tab);
    let on = location_rows(&frame(&mut palette, 120, 40));
    assert_documented_order(&on);

    change_setting(&mut palette, SETTINGS[1], KeyCode::Right);
    let frame = frame(&mut palette, 120, 40);
    assert_eq!(query_text(&frame), "@", "{frame:#?}");
    let mut off = location_rows(&frame);
    let last = off.pop();
    assert_documented_order(&off);
    assert_eq!(
        last,
        Some(LocationLine {
            indented: false,
            label: "/synthetic/gone".to_string(),
            detail: "missing · folder no longer exists".to_string(),
            count: 1,
        })
    );
}

#[test]
fn orphans_last_off_lists_an_orphaned_session_by_its_age() {
    let histories = Histories::new();
    histories.claude(CLAUDE_RETRY, "/usr", "Live work");
    histories.claude_records(
        CLAUDE_GONE,
        "/synthetic/gone",
        &[json!({
            "type": "user",
            "message": { "content": "Orphaned work" },
            "timestamp": "2026-10-21T10:00:00Z",
        })],
    );
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let titles = ["Live", "Orphaned"];

    let on = frame(&mut palette, 80, 24);
    let rows = rows_with(&on, &titles);
    assert!(rows[1].starts_with("│ Orphaned"), "{on:#?}");

    change_setting(&mut palette, SETTINGS[2], KeyCode::Right);
    let off = frame(&mut palette, 80, 24);
    let rows = rows_with(&off, &titles);
    assert!(rows[0].starts_with("│ Orphaned"), "{off:#?}");
}

#[test]
fn the_accent_cycles_through_every_theme_and_colours_the_palette() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    ctrl(&mut palette, 's');
    for _ in 0..3 {
        press(&mut palette, KeyCode::Down);
    }
    for accent in ["Cyan", "Amber", "Violet", "Sky", "Mono", "Sage"] {
        press(&mut palette, KeyCode::Right);
        assert_eq!(setting(&frame(&mut palette, 80, 24), SETTINGS[3]), accent);
    }
    press(&mut palette, KeyCode::Left);
    assert_eq!(setting(&frame(&mut palette, 80, 24), SETTINGS[3]), "Mono");

    // The accent is saved, so a fresh history starts from Sage again.
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    change_setting(&mut palette, SETTINGS[3], KeyCode::Right);
    let cyan = draw(&mut palette, 80, 24);
    assert!(
        cyan[(2, 3)].modifier.contains(Modifier::REVERSED),
        "{cyan:?}"
    );
    assert_eq!(selected(&cyan), Some("Tidy"));

    change_setting(&mut palette, SETTINGS[3], KeyCode::Right);
    keys(&mut palette, "#claude ");
    let amber = draw(&mut palette, 80, 24);
    assert_eq!(amber[(2, 1)].symbol(), "›");
    assert_eq!(amber[(2, 1)].fg, Color::Indexed(214));
    assert_eq!(amber[(4, 1)].symbol(), "#");
    assert_eq!(amber[(4, 1)].fg, Color::Indexed(214));
}

#[test]
fn no_color_overrides_the_chosen_accent_and_says_so() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Mono, Path::new("/"));
    change_setting(&mut palette, SETTINGS[3], KeyCode::Right);
    change_setting(&mut palette, SETTINGS[3], KeyCode::Right);

    let buffer = draw(&mut palette, 80, 24);
    for cell in buffer.content() {
        assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset), "{cell:?}");
    }
    assert!(buffer[(2, 3)].modifier.contains(Modifier::REVERSED));
    assert_eq!(selected(&buffer), Some("Tidy"));

    ctrl(&mut palette, 's');
    let screen = draw(&mut palette, 80, 24);
    for cell in screen.content() {
        assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset), "{cell:?}");
    }
    assert_eq!(
        setting(&lines(&screen), SETTINGS[3]),
        "Amber · off: NO_COLOR"
    );
}

#[test]
fn the_icon_set_cycles_and_shows_sample_glyphs() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    ctrl(&mut palette, 's');
    for _ in 0..4 {
        press(&mut palette, KeyCode::Down);
    }
    for set in ["Unicode ▣ ⎇ ▢ ⊘", "Text tags git wt dir gone", "None"] {
        press(&mut palette, KeyCode::Right);
        assert_eq!(
            words(&setting(&frame(&mut palette, 80, 24), SETTINGS[4])),
            set
        );
    }
    press(&mut palette, KeyCode::Right);
    let nerd_font = setting(&frame(&mut palette, 80, 24), SETTINGS[4]);
    assert!(nerd_font.starts_with("Nerd Font"), "{nerd_font}");
}

/// The palette over fixture F, whose icon set is Nerd Font, with the icon
/// set `steps` after it.
fn icons_palette(histories: &Histories, steps: usize) -> Palette {
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    for _ in 0..steps {
        change_setting(&mut palette, SETTINGS[4], KeyCode::Right);
    }
    palette
}

/// Whether some row of `frame` contains `text`.
fn shows(frame: &[String], text: &str) -> bool {
    frame.iter().any(|row| row.contains(text))
}

#[test]
fn unicode_and_text_tag_icons_mark_locations_and_results() {
    let (histories, _live) = locations_fixture();
    let mut palette = icons_palette(&histories, 0);
    for [repo, worktree, folder, missing] in [
        ["▣ ", "⎇ ", "▢ ", "⊘ "],
        ["git  ", "wt   ", "dir  ", "gone "],
    ] {
        change_setting(&mut palette, SETTINGS[4], KeyCode::Right);
        ctrl(&mut palette, 'u');
        let results = frame(&mut palette, 120, 40);
        let usr = rows_with(&results, &["Usr"]);
        assert!(usr[0].contains(&format!("{folder}/usr")), "{results:#?}");
        let gone = rows_with(&results, &["Gone"]);
        let after_label = gone[0].split("orphaned").nth(1).unwrap();
        assert!(after_label.contains(missing), "{results:#?}");

        keys(&mut palette, "@/");
        let list = frame(&mut palette, 120, 40);
        assert!(shows(&list, &format!("│ {repo}lint-kit ")), "{list:#?}");
        assert!(
            shows(&list, &format!("│    {worktree}billing-retry ")),
            "{list:#?}"
        );
        assert!(shows(&list, &format!("│ {folder}/usr ")), "{list:#?}");
        assert!(
            shows(&list, &format!("│ {missing}/synthetic/gone ")),
            "{list:#?}"
        );
    }
}

#[test]
fn no_icons_leave_no_gap_where_the_icon_was() {
    let (histories, _live) = locations_fixture();
    let mut nerd_font = icons_palette(&histories, 0);
    let mut none = icons_palette(&histories, 3);

    // A result's place starts in the column its icon would.
    let with_icon = frame(&mut nerd_font, 120, 40);
    let without = frame(&mut none, 120, 40);
    let usr_with = rows_with(&with_icon, &["Usr"])[0];
    let usr_without = rows_with(&without, &["Usr"])[0];
    let icon_at = usr_with.chars().position(|c| c == '\u{f07b}').unwrap();
    let place_at = usr_without
        .find("/usr")
        .map(|at| usr_without[..at].chars().count());
    assert_eq!(place_at, Some(icon_at), "{without:#?}");

    // The selected session ran in /usr; its preview starts there.
    let buffer = draw(&mut none, 120, 40);
    let column = preview_column(&buffer).unwrap();
    let dir_line = (3..38)
        .find(|&y| cells(&buffer, y, column + 1..119).trim() == "/usr")
        .unwrap_or_else(|| panic!("{:#?}", lines(&buffer)));
    assert_eq!(cells(&buffer, dir_line, column + 1..column + 6), " /usr");

    press(&mut none, KeyCode::Right);
    let reading = frame(&mut none, 120, 40);
    assert!(reading[2].contains(" ago · /usr"), "{reading:#?}");
    press(&mut none, KeyCode::Esc);

    keys(&mut none, "@/");
    let list = frame(&mut none, 120, 40);
    for label in ["lint-kit", "acme-api", "/usr", "/synthetic/gone"] {
        assert!(shows(&list, &format!("│ {label} ")), "{label}: {list:#?}");
    }
    for label in ["billing-retry", "docs-site"] {
        assert!(
            shows(&list, &format!("│    {label} ")),
            "{label}: {list:#?}"
        );
    }
}

/// Each setting's value once `change_every_setting` has run.
const CHANGED: [&str; 5] = ["off", "off", "off", "Cyan", "Unicode ▣ ⎇ ▢ ⊘"];

/// Changes every setting from its default to the value `CHANGED` names.
fn change_every_setting(palette: &mut Palette) {
    for label in SETTINGS {
        change_setting(palette, label, KeyCode::Right);
    }
}

/// Checks that `palette` opens with every setting as `CHANGED` names it,
/// and draws by them.
fn assert_changed(palette: &mut Palette) {
    let buffer = draw(palette, 80, 24);
    let rows = lines(&buffer);
    assert!(
        buffer[(2, 3)].modifier.contains(Modifier::REVERSED),
        "{rows:#?}"
    );
    assert!(shows(&rows, "▢ /usr"), "{rows:#?}");
    ctrl(palette, 's');
    let screen = frame(palette, 80, 24);
    let values: Vec<String> = SETTINGS
        .iter()
        .map(|label| words(&setting(&screen, label)))
        .collect();
    assert_eq!(values, CHANGED, "{screen:#?}");
}

#[test]
fn settings_survive_a_relaunch_and_a_deleted_index() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    change_every_setting(&mut palette);

    let mut relaunched = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert_changed(&mut relaunched);

    fs::remove_file(histories.data().index()).unwrap();
    let mut rebuilt = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    assert_changed(&mut rebuilt);
}

#[test]
fn preferences_from_before_settings_keep_their_hides_and_take_settings() {
    let histories = Histories::three();
    let prefs = histories.data().prefs();
    fs::create_dir_all(prefs.parent().unwrap()).unwrap();
    let old = rusqlite::Connection::open(&prefs).unwrap();
    old.execute_batch(
        "CREATE TABLE hidden (provider TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY (provider, id));
         PRAGMA user_version = 1;",
    )
    .unwrap();
    old.execute(
        "INSERT INTO hidden (provider, id) VALUES ('claude', ?1)",
        [CLAUDE_RETRY],
    )
    .unwrap();
    drop(old);

    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let opened = frame(&mut palette, 80, 24);
    assert!(rows_with(&opened, &["Retry"]).is_empty(), "{opened:#?}");
    ctrl(&mut palette, 's');
    let screen = frame(&mut palette, 80, 24);
    assert_eq!(setting(&screen, SETTINGS[3]), "Sage", "{screen:#?}");
    press(&mut palette, KeyCode::Right);
    let changed = frame(&mut palette, 80, 24);
    assert_eq!(setting(&changed, SETTINGS[0]), "off", "{changed:#?}");

    let mut relaunched = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let rows = frame(&mut relaunched, 80, 24);
    assert!(rows_with(&rows, &["Retry"]).is_empty(), "{rows:#?}");
    ctrl(&mut relaunched, 's');
    assert_eq!(setting(&frame(&mut relaunched, 80, 24), SETTINGS[0]), "off");
}

#[cfg(unix)]
#[test]
fn a_setting_that_cannot_be_saved_says_so_and_stays_as_it_was() {
    let histories = Histories::three();
    let catalog = histories.catalog();
    let dir = histories.data().prefs().parent().unwrap().to_path_buf();
    assert!(!histories.data().prefs().exists());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    // Root may write anywhere, so there is nothing to fail.
    if fs::write(dir.join("probe"), "").is_ok() {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let mut palette = Palette::new(catalog, Colour::Accent, Path::new("/"));
    ctrl(&mut palette, 's');
    press(&mut palette, KeyCode::Right);
    let frame = frame(&mut palette, 80, 24);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        frame[22].contains("can't use the preferences"),
        "{frame:#?}"
    );
    assert_eq!(setting(&frame, SETTINGS[0]), "on", "{frame:#?}");
}

#[test]
fn settings_give_way_in_narrow_and_short_terminals() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    change_setting(&mut palette, SETTINGS[4], KeyCode::Right);
    change_setting(&mut palette, SETTINGS[4], KeyCode::Right);
    ctrl(&mut palette, 's');
    let data = histories.data();

    // Every label and value fits whole at 60 columns; only the paths are cut.
    let narrow = frame(&mut palette, 60, 20);
    assert_eq!(setting(&narrow, SETTINGS[1]), "on", "{narrow:#?}");
    assert_eq!(
        words(&setting(&narrow, SETTINGS[4])),
        "Text tags git wt dir gone"
    );
    for (label, path) in [("index", data.index()), ("preferences", data.prefs())] {
        let row = narrow
            .iter()
            .find(|row| row.starts_with(&format!("│ {label} ")))
            .unwrap_or_else(|| panic!("{narrow:#?}"));
        assert!(row.contains('…'), "{narrow:#?}");
        assert!(shows_path(&narrow, label, &path), "{narrow:#?}");
    }

    let short = frame(&mut palette, 48, 12);
    for label in &SETTINGS[..4] {
        assert!(!setting(&short, label).is_empty(), "{short:#?}");
    }
    assert!(short[7].starts_with("│ Location icons"), "{short:#?}");
    assert!(short[8].starts_with("│ index "), "{short:#?}");
    assert!(short[9].starts_with("│ preferences "), "{short:#?}");

    let too_small = frame(&mut palette, 47, 12);
    assert!(
        too_small
            .iter()
            .any(|row| row.trim() == "terminal too small"),
        "{too_small:#?}"
    );
}

/// A directory holding an executable stand-in for each of `programs`, for
/// the palette to find them on.
fn bin_with(programs: &[&str]) -> TempDir {
    let bin = tempfile::tempdir().unwrap();
    for program in programs {
        let path = bin.path().join(program);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// The palette over `histories`, finding providers only in `bin`,
/// launched from `launch_dir`.
fn finding_in(histories: &Histories, bin: &Path, launch_dir: &Path) -> Palette {
    Palette::new(
        catalog_finding_in(histories, bin),
        Colour::Accent,
        launch_dir,
    )
}

/// The catalog over `histories`, finding providers only in `bin`.
fn catalog_finding_in(histories: &Histories, bin: &Path) -> Catalog {
    histories
        .catalog()
        .finding_programs_in(bin.as_os_str().to_owned())
}

/// What the dialog's line labelled `label` says after it, or `None` when
/// no line starts with that label.
fn field(frame: &[String], label: &str) -> Option<String> {
    frame.iter().find_map(|row| {
        let inner = row.trim_start_matches('│').trim_end_matches('│');
        let rest = inner.strip_prefix(&format!(" {label} "))?;
        Some(rest.trim().to_string())
    })
}

#[test]
fn enter_shows_the_claude_command_and_directory_and_esc_goes_back() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Down);
    let before = frame(&mut palette, 120, 40);

    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);
    let dialog = frame(&mut palette, 120, 40);
    assert!(
        dialog[1].starts_with("│ Resume \"Retry the billing job\""),
        "{dialog:#?}"
    );
    assert_eq!(
        field(&dialog, "directory").as_deref(),
        Some("/usr"),
        "{dialog:#?}"
    );
    assert_eq!(
        field(&dialog, "command"),
        Some(format!("claude --resume {CLAUDE_RETRY}")),
        "{dialog:#?}"
    );

    assert_eq!(press(&mut palette, KeyCode::Esc), Step::Continue);
    assert_eq!(frame(&mut palette, 120, 40), before);
}

#[test]
fn enter_shows_the_codex_command_and_directory() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Enter);
    let dialog = frame(&mut palette, 120, 40);
    assert_eq!(
        field(&dialog, "directory").as_deref(),
        Some("/usr/bin"),
        "{dialog:#?}"
    );
    assert_eq!(
        field(&dialog, "command"),
        Some(format!("codex resume {CODEX} -C /usr/bin")),
        "{dialog:#?}"
    );
}

#[test]
fn enter_shows_the_pi_command_and_directory() {
    let histories = Histories::three();
    histories.pi(PI, "/usr", "Sort the inbox");
    let bin = bin_with(&["claude", "codex", "pi"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    keys(&mut palette, "#pi ");
    press(&mut palette, KeyCode::Enter);
    let dialog = frame(&mut palette, 120, 40);
    assert!(
        dialog[1].starts_with("│ Resume \"Sort the inbox\""),
        "{dialog:#?}"
    );
    assert_eq!(field(&dialog, "directory").as_deref(), Some("/usr"));
    assert_eq!(
        field(&dialog, "command"),
        Some(format!("pi --session {PI}")),
        "{dialog:#?}"
    );
    assert_eq!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(ResumeCommand {
            program: "pi".to_string(),
            args: vec!["--session".to_string(), PI.to_string()],
            cwd: PathBuf::from("/usr"),
        })
    );
}

#[test]
fn a_second_enter_gives_the_command_to_start() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);
    assert_eq!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), CLAUDE_RETRY.to_string()],
            cwd: PathBuf::from("/usr"),
        })
    );
}

#[test]
fn enter_while_reading_opens_the_dialog_and_esc_steps_back() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Down);
    let palette_frame = frame(&mut palette, 80, 24);
    press(&mut palette, KeyCode::Right);
    let reading = frame(&mut palette, 80, 24);

    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);
    let dialog = frame(&mut palette, 80, 24);
    assert!(
        dialog[1].starts_with("│ Resume \"Retry the billing job\""),
        "{dialog:#?}"
    );
    assert_eq!(field(&dialog, "directory").as_deref(), Some("/usr"));

    press(&mut palette, KeyCode::Esc);
    assert_eq!(frame(&mut palette, 80, 24), reading);
    press(&mut palette, KeyCode::Esc);
    assert_eq!(frame(&mut palette, 80, 24), palette_frame);

    press(&mut palette, KeyCode::Enter);
    assert_eq!(ctrl(&mut palette, 'c'), Step::Quit);
}

/// The footer of a frame `height` rows tall: the row above the bottom border.
fn footer(frame: &[String]) -> &str {
    &frame[frame.len() - 2]
}

#[test]
fn an_orphaned_claude_session_resumes_in_the_launch_directory_only_after_c() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let launch = live_dir();
    let launched = launch.path().display().to_string();
    let mut palette = finding_in(&histories, bin.path(), launch.path());
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);

    let dialog = frame(&mut palette, 240, 40);
    let orphaned = field(&dialog, "orphaned").unwrap_or_else(|| panic!("{dialog:#?}"));
    assert!(orphaned.contains("The folder is gone"), "{orphaned}");
    assert!(
        orphaned.contains(&format!("c resumes in {launched} instead")),
        "{orphaned}"
    );
    assert!(footer(&dialog).contains("c here"), "{dialog:#?}");
    assert!(!footer(&dialog).contains("↵ resume"), "{dialog:#?}");
    assert_eq!(field(&dialog, "warning"), None, "{dialog:#?}");
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);

    press(&mut palette, KeyCode::Char('c'));
    let elsewhere = frame(&mut palette, 240, 40);
    assert_eq!(
        field(&elsewhere, "directory"),
        Some(format!("{launched} (launch directory)")),
        "{elsewhere:#?}"
    );
    let warning = field(&elsewhere, "warning").unwrap_or_else(|| panic!("{elsewhere:#?}"));
    assert!(warning.contains("may not find this one"), "{warning}");
    assert!(footer(&elsewhere).contains("↵ resume"), "{elsewhere:#?}");
    assert!(!footer(&elsewhere).contains("c here"), "{elsewhere:#?}");

    press(&mut palette, KeyCode::Char('x'));
    assert_eq!(frame(&mut palette, 240, 40), elsewhere);
    assert_eq!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), CLAUDE_GONE.to_string()],
            cwd: launch.path().to_path_buf(),
        })
    );
}

#[test]
fn an_orphaned_codex_session_resumes_elsewhere_without_a_warning() {
    let histories = Histories::new();
    histories.codex(CODEX, "/synthetic/gone", "Tidy the lint config");
    let bin = bin_with(&["claude", "codex"]);
    let launch = live_dir();
    let launched = launch.path().display().to_string();
    let mut palette = finding_in(&histories, bin.path(), launch.path());
    press(&mut palette, KeyCode::Enter);
    press(&mut palette, KeyCode::Char('c'));

    let dialog = frame(&mut palette, 240, 40);
    assert_eq!(field(&dialog, "warning"), None, "{dialog:#?}");
    assert_eq!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(ResumeCommand {
            program: "codex".to_string(),
            args: vec![
                "resume".to_string(),
                CODEX.to_string(),
                "-C".to_string(),
                launched,
            ],
            cwd: launch.path().to_path_buf(),
        })
    );
}

#[test]
fn resuming_elsewhere_says_when_the_launch_directory_is_gone_too() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/synthetic/also-gone"));
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);
    press(&mut palette, KeyCode::Char('c'));

    let dialog = frame(&mut palette, 120, 40);
    assert!(
        dialog.iter().any(
            |row| row.contains("orphaned") && row.contains("The launch directory is gone too.")
        ),
        "{dialog:#?}"
    );
    assert!(!footer(&dialog).contains("↵ resume"), "{dialog:#?}");
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);
}

#[cfg(unix)]
#[test]
fn r_checks_an_unavailable_folder_again_and_updates_the_dialog() {
    let live = live_dir();
    let locked = live.path().join("locked");
    fs::create_dir(&locked).unwrap();
    let histories = Histories::new();
    histories.claude(CLAUDE_RETRY, locked.to_str().unwrap(), "Locked work");
    let bin = bin_with(&["claude", "codex"]);
    let lock = Locked::new(&locked);
    // Root reads the folder anyway, so it is never unavailable.
    if !lock.denies_access() {
        return;
    }
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Enter);

    let dialog = frame(&mut palette, 120, 40);
    let unavailable = field(&dialog, "unavailable").unwrap_or_else(|| panic!("{dialog:#?}"));
    assert!(unavailable.contains("can't be inspected"), "{unavailable}");
    assert!(footer(&dialog).contains("c here"), "{dialog:#?}");
    assert!(footer(&dialog).contains("r check again"), "{dialog:#?}");
    assert!(!footer(&dialog).contains("↵ resume"), "{dialog:#?}");
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);

    press(&mut palette, KeyCode::Char('r'));
    let checked = frame(&mut palette, 120, 40);
    assert!(
        checked
            .iter()
            .any(|row| row.contains("Checked again: still can't inspect it.")),
        "{checked:#?}"
    );

    drop(lock);
    press(&mut palette, KeyCode::Char('r'));
    let available = frame(&mut palette, 120, 40);
    assert_eq!(field(&available, "unavailable"), None, "{available:#?}");
    assert!(
        !available.iter().any(|row| row.contains("Checked again")),
        "{available:#?}"
    );
    assert!(
        !footer(&available).contains("r check again"),
        "{available:#?}"
    );
    assert!(footer(&available).contains("↵ resume"), "{available:#?}");
    assert_eq!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(ResumeCommand {
            program: "claude".to_string(),
            args: vec!["--resume".to_string(), CLAUDE_RETRY.to_string()],
            cwd: locked,
        })
    );
}

#[test]
fn a_running_session_shows_a_warning_and_still_resumes() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = finding_in(&histories, bin.path(), Path::new("/"));
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);
    let dialog = frame(&mut palette, 120, 40);
    assert_eq!(field(&dialog, "running"), None, "{dialog:#?}");

    let sessions = histories.roots().claude.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let record = json!({ "pid": 4242, "sessionId": CLAUDE_RETRY, "cwd": "/usr", "status": "idle" });
    fs::write(sessions.join("4242.json"), record.to_string()).unwrap();
    press(&mut palette, KeyCode::Esc);
    press(&mut palette, KeyCode::Enter);
    let dialog = frame(&mut palette, 120, 40);
    let running = field(&dialog, "running").unwrap_or_else(|| panic!("{dialog:#?}"));
    assert!(running.contains("lists this session as open"), "{running}");
    assert!(footer(&dialog).contains("↵ resume"), "{dialog:#?}");
    assert!(matches!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(_)
    ));
}

/// The dialog on the Claude Code `/usr` session, finding providers in `bin`.
fn claude_dialog(histories: &Histories, bin: &Path) -> Palette {
    let mut palette = finding_in(histories, bin, Path::new("/"));
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Enter);
    palette
}

#[test]
fn a_provider_not_on_path_says_so_and_cannot_resume() {
    let histories = Histories::three();
    let empty = bin_with(&[]);
    let mut palette = claude_dialog(&histories, empty.path());
    let dialog = frame(&mut palette, 120, 40);
    assert_eq!(
        field(&dialog, "not found").as_deref(),
        Some("claude isn't on PATH, so it can't start."),
        "{dialog:#?}"
    );
    assert!(!footer(&dialog).contains("↵ resume"), "{dialog:#?}");
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);

    let installed = bin_with(&["claude"]);
    let mut palette = claude_dialog(&histories, installed.path());
    assert_eq!(field(&frame(&mut palette, 120, 40), "not found"), None);
    assert!(matches!(
        press(&mut palette, KeyCode::Enter),
        Step::Resume(_)
    ));
}

#[cfg(unix)]
#[test]
fn a_provider_that_is_not_executable_is_not_found() {
    let histories = Histories::three();
    let bin = bin_with(&["claude"]);
    fs::set_permissions(bin.path().join("claude"), fs::Permissions::from_mode(0o644)).unwrap();
    let mut palette = claude_dialog(&histories, bin.path());
    let dialog = frame(&mut palette, 120, 40);
    assert!(field(&dialog, "not found").is_some(), "{dialog:#?}");
    assert_eq!(press(&mut palette, KeyCode::Enter), Step::Continue);
}

#[test]
fn the_dialog_keeps_one_border_wraps_long_paths_and_has_no_colour_in_mono() {
    let histories = Histories::three();
    let bin = bin_with(&["claude", "codex"]);
    let mut palette = claude_dialog(&histories, bin.path());
    for (width, height) in [(120, 40), (80, 24), (60, 20)] {
        let dialog = frame(&mut palette, width, height);
        let corners: usize = dialog.iter().map(|row| row.matches('╭').count()).sum();
        assert_eq!(corners, 1, "{dialog:#?}");
        assert_eq!(field(&dialog, "directory").as_deref(), Some("/usr"));
    }
    let too_small = frame(&mut palette, 47, 11);
    let shown: Vec<&str> = too_small
        .iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(shown, ["terminal too small", "need at least 48×12"]);

    let long = format!("/synthetic/{}end", "abcdefghij/".repeat(13));
    let histories = Histories::new();
    histories.claude(CLAUDE_GONE, &long, "Release notes draft");
    let mut palette = dialog_on_first(&histories, bin.path(), Colour::Accent);
    let dialog = frame(&mut palette, 60, 20);
    let start = dialog
        .iter()
        .position(|row| row.starts_with("│ directory "))
        .unwrap_or_else(|| panic!("{dialog:#?}"));
    let pieces: Vec<&str> = dialog[start..]
        .iter()
        .enumerate()
        .take_while(|(n, row)| *n == 0 || row.starts_with(&format!("│{}", " ".repeat(14))))
        .map(|(_, row)| {
            row.trim_matches('│')
                .trim()
                .trim_start_matches("directory")
                .trim()
        })
        .collect();
    assert!(pieces.len() > 1, "{dialog:#?}");
    assert_eq!(pieces.concat(), long, "{dialog:#?}");

    let mut palette = dialog_on_first(&histories, bin.path(), Colour::Mono);
    let buffer = draw(&mut palette, 80, 24);
    for cell in buffer.content() {
        assert_eq!(cell.fg, Color::Reset, "{cell:?}");
        assert_eq!(cell.bg, Color::Reset, "{cell:?}");
    }
}

/// The dialog on the first session listed, drawn in `colour`, finding
/// providers in `bin`.
fn dialog_on_first(histories: &Histories, bin: &Path, colour: Colour) -> Palette {
    let mut palette = Palette::new(catalog_finding_in(histories, bin), colour, Path::new("/"));
    press(&mut palette, KeyCode::Enter);
    palette
}

#[test]
fn the_footer_names_enter_for_resume_under_the_results_and_while_reading() {
    let histories = Histories::three();
    let mut palette = Palette::new(histories.catalog(), Colour::Accent, Path::new("/"));
    let results = frame(&mut palette, 120, 40);
    assert!(
        footer(&results).contains("↑↓ move  ↵ resume"),
        "{results:#?}"
    );
    press(&mut palette, KeyCode::Right);
    let reading = frame(&mut palette, 120, 40);
    assert!(
        footer(&reading).contains("↑↓ scroll  ↵ resume"),
        "{reading:#?}"
    );
}

/// Steps `stepper` until its pass has ended.
fn finish(stepper: &mut Stepper) {
    while stepper.step() {}
}

const CLAUDE_NEW: &str = "aaaaaaaa-0000-4000-8000-000000000404";

#[test]
fn a_warm_index_shows_its_sessions_before_the_indexer_reads_anything() {
    let histories = Histories::three();
    histories.catalog();
    histories.claude(CLAUDE_NEW, "/usr", "Draft the onboarding guide");
    let (mut palette, mut stepper) = histories.palette_indexing();

    let first = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&first, &TITLES).len(), 3, "{first:#?}");
    assert!(rows_with(&first, &["Draft"]).is_empty(), "{first:#?}");
    assert!(!first[1].contains("indexing"), "{first:#?}");

    finish(&mut stepper);
    let after = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&after, &["Draft"]).len(), 1, "{after:#?}");
}

/// Appends `records` to Claude Code session `id`, run in `cwd`, each given
/// the session's `cwd` and the timestamp `Histories::claude` gives.
fn append_claude(histories: &Histories, id: &str, cwd: &str, records: &[Value]) {
    let path = histories
        .dir
        .path()
        .join("claude/projects/-synthetic-palette")
        .join(format!("{id}.jsonl"));
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    for record in records {
        let mut record = record.clone();
        record["cwd"] = json!(cwd);
        record["timestamp"] = json!("2026-10-20T10:00:00Z");
        writeln!(file, "{record}").unwrap();
    }
}

#[test]
fn an_update_keeps_the_open_conversation_where_it_was_read() {
    let histories = Histories::three();
    long_session(&histories);
    histories.catalog();
    let (mut palette, mut stepper) = histories.palette_indexing();
    // Tidy, Retry, then the long session, which isn't the first row.
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Right);
    for _ in 0..4 {
        press(&mut palette, KeyCode::Down);
    }
    let before = frame(&mut palette, 80, 24);
    assert!(before[1].contains("turn 0 asks"), "{before:#?}");

    append_claude(
        &histories,
        CLAUDE_FULL,
        "/usr",
        &[json!({ "type": "user", "message": { "content": "turn 40 appended" } })],
    );
    finish(&mut stepper);
    let after = frame(&mut palette, 80, 24);
    assert_eq!(after[1], before[1], "{after:#?}");
    assert_eq!(
        conversation_body(&after)[0],
        conversation_body(&before)[0],
        "{after:#?}"
    );
    press(&mut palette, KeyCode::End);
    let end = conversation_body(&frame(&mut palette, 80, 24));
    assert!(end.last().unwrap().contains("turn 40 appended"), "{end:#?}");

    press(&mut palette, KeyCode::Esc);
    let results = frame(&mut palette, 80, 24);
    assert!(
        results.iter().any(|row| row.contains("turn 40 appended")),
        "{results:#?}"
    );
}

/// The row the selection bar is on, as `lines` reads it.
fn selected_row(buffer: &Buffer) -> Option<String> {
    let frame = lines(buffer);
    (3..buffer.area.height.saturating_sub(2))
        .find(|&y| {
            buffer[(2, y)]
                .modifier
                .intersects(Modifier::BOLD | Modifier::REVERSED)
        })
        .map(|y| frame[usize::from(y)].clone())
}

#[test]
fn an_update_adds_a_session_and_keeps_the_query_and_the_selected_session() {
    let histories = Histories::three();
    let export = "aaaaaaaa-0000-4000-8000-000000000405";
    histories.claude(export, "/usr", "Billing export cleanup");
    histories.claude(
        "aaaaaaaa-0000-4000-8000-000000000406",
        "/usr",
        "Billing invoice totals",
    );
    histories.catalog();
    let (mut palette, mut stepper) = histories.palette_indexing();
    keys(&mut palette, "billing");
    // Retry, Billing export, then Billing invoice.
    press(&mut palette, KeyCode::Down);
    press(&mut palette, KeyCode::Down);
    let before = selected_row(&draw(&mut palette, 80, 24)).unwrap();
    assert!(before.starts_with("│ Billing invoice"), "{before}");

    // A day later than the rest, so it sorts above them.
    histories.claude_records(
        "aaaaaaaa-0000-4000-8000-000000000407",
        "/usr",
        &[json!({
            "type": "user",
            "message": { "content": "Billing ledger audit" },
            "timestamp": "2026-10-21T10:00:00Z",
        })],
    );
    finish(&mut stepper);
    let buffer = draw(&mut palette, 80, 24);
    let after = lines(&buffer);
    assert!(after[3].starts_with("│ Billing ledger"), "{after:#?}");
    assert_eq!(query_text(&after), "billing", "{after:#?}");
    let selected = selected_row(&buffer).unwrap();
    assert!(selected.starts_with("│ Billing invoice"), "{after:#?}");

    // With the selected session gone, the bar stays on its row.
    press(&mut palette, KeyCode::Up);
    let before = selected_row(&draw(&mut palette, 80, 24)).unwrap();
    assert!(before.starts_with("│ Billing export"), "{before}");
    fs::remove_file(
        histories
            .dir
            .path()
            .join("claude/projects/-synthetic-palette")
            .join(format!("{export}.jsonl")),
    )
    .unwrap();
    let (indexer, mut stepper) = Indexer::stepped(&histories.roots(), &histories.data());
    let mut palette = palette.indexing(indexer);
    finish(&mut stepper);
    let buffer = draw(&mut palette, 80, 24);
    let after = lines(&buffer);
    assert!(after[5].starts_with("│ Billing invoice"), "{after:#?}");
    assert_eq!(
        selected_row(&buffer).as_ref(),
        Some(&after[5]),
        "{after:#?}"
    );
}

/// What `palette` draws into an 80×24 terminal at `at`.
fn frame_at(palette: &mut Palette, at: Instant) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| palette.render(frame, at)).unwrap();
    lines(terminal.backend().buffer())
}

#[test]
fn indexing_shows_on_the_query_line_while_a_pass_runs_and_clears_after() {
    let histories = Histories::new();
    histories.many(40);
    let (mut palette, mut stepper) = histories.palette_indexing();
    assert!(stepper.step());

    let start = Instant::now();
    let first = frame_at(&mut palette, start);
    assert!(!first[1].contains("indexing"), "{first:#?}");
    assert_eq!(palette.redraw_at(), Some(start + Duration::from_millis(50)));
    let soon = frame_at(&mut palette, start + Duration::from_millis(50));
    assert!(!soon[1].contains("indexing"), "{soon:#?}");
    let later = frame_at(&mut palette, start + Duration::from_millis(200));
    assert!(later[1].contains("indexing · "), "{later:#?}");
    assert!(palette.redraw_at().is_some());

    finish(&mut stepper);
    let ended = start + Duration::from_millis(250);
    let done = frame_at(&mut palette, ended);
    assert!(!done[1].contains("indexing"), "{done:#?}");
    assert_eq!(palette.redraw_at(), Some(ended + REFRESH_INTERVAL));
}

/// The match count at the end of the query line, such as `32 / 32`.
fn count(frame: &[String]) -> &str {
    let line = frame[1].trim_end_matches('│').trim_end();
    let at = line.rfind(" / ").unwrap();
    let start = line[..at].rfind(' ').unwrap() + 1;
    &line[start..]
}

#[test]
fn a_first_launch_shows_each_batch_as_it_is_indexed() {
    let histories = Histories::new();
    histories.many(40);
    let (mut palette, mut stepper) = histories.palette_indexing();

    let first = frame(&mut palette, 80, 24);
    assert_eq!(
        results(&first[3]).trim(),
        "Indexing histories…",
        "{first:#?}"
    );
    assert!(!first.iter().any(|row| row.contains("No sessions found.")));
    assert!(stepper.step());
    assert_eq!(count(&frame(&mut palette, 80, 24)), "32 / 32");
    assert!(stepper.step());
    assert_eq!(count(&frame(&mut palette, 80, 24)), "40 / 40");
    assert!(!stepper.step());

    let empty = Histories::new();
    let (mut palette, mut stepper) = empty.palette_indexing();
    let first = frame(&mut palette, 80, 24);
    assert_eq!(
        results(&first[3]).trim(),
        "Indexing histories…",
        "{first:#?}"
    );
    finish(&mut stepper);
    let done = frame(&mut palette, 80, 24);
    assert_eq!(results(&done[3]).trim(), "No sessions found.", "{done:#?}");
}

#[test]
fn resuming_mid_index_drops_the_indexer_at_once_and_leaves_the_index_consistent() {
    let histories = Histories::three();
    histories.catalog();
    histories.many(300);
    let bin = bin_with(&["claude", "codex"]);
    let catalog = Catalog::cached(&histories.roots(), &histories.data())
        .unwrap()
        .finding_programs_in(bin.path().as_os_str().to_owned());
    let indexer = Indexer::start(&histories.roots(), &histories.data());
    let mut palette = Palette::new(catalog, Colour::Accent, Path::new("/")).indexing(indexer);

    press(&mut palette, KeyCode::Enter);
    let step = press(&mut palette, KeyCode::Enter);
    assert!(matches!(step, Step::Resume(_)), "{step:?}");
    // As `run` does before the command replaces the process.
    drop(palette);

    let listed = list(&histories.roots(), &histories.data()).unwrap();
    let rebuilt = rebuild(&histories.roots(), &histories.data()).unwrap();
    let ids = |sessions: &[yapkeeper::Listed]| -> Vec<String> {
        sessions
            .iter()
            .map(|listed| listed.session.id.clone())
            .collect()
    };
    assert_eq!(ids(&listed.sessions), ids(&rebuilt.sessions));
    assert_eq!(listed.sessions.len(), 303);
}

#[test]
fn a_refresh_runs_once_the_interval_has_passed_since_the_last_pass() {
    let histories = Histories::three();
    histories.catalog();
    let (mut palette, mut stepper) = histories.palette_indexing();
    finish(&mut stepper);
    let ended = Instant::now();
    frame_at(&mut palette, ended);
    assert_eq!(palette.redraw_at(), Some(ended + REFRESH_INTERVAL));
    histories.claude(CLAUDE_NEW, "/usr", "Draft the onboarding guide");

    let early = ended + REFRESH_INTERVAL - Duration::from_millis(1);
    frame_at(&mut palette, early);
    assert!(!stepper.step());
    let before = frame_at(&mut palette, early);
    assert!(rows_with(&before, &["Draft"]).is_empty(), "{before:#?}");

    frame_at(&mut palette, ended + REFRESH_INTERVAL);
    finish(&mut stepper);
    let after = frame_at(&mut palette, ended + REFRESH_INTERVAL);
    assert_eq!(rows_with(&after, &["Draft"]).len(), 1, "{after:#?}");
}

#[test]
fn ctrl_r_refreshes_at_once_and_not_again_while_a_pass_runs() {
    let histories = Histories::new();
    histories.many(40);
    let (mut palette, mut stepper) = histories.palette_indexing();
    // The pass has planned its files and committed the newest 32.
    assert!(stepper.step());
    histories.claude(CLAUDE_NEW, "/usr", "Draft the onboarding guide");
    ctrl(&mut palette, 'r');
    finish(&mut stepper);
    assert!(!stepper.step());
    let during = frame(&mut palette, 80, 24);
    assert!(rows_with(&during, &["Draft"]).is_empty(), "{during:#?}");

    ctrl(&mut palette, 'r');
    finish(&mut stepper);
    let after = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&after, &["Draft"]).len(), 1, "{after:#?}");
}

#[test]
fn a_file_appended_to_during_a_pass_shows_after_the_next_pass() {
    let histories = Histories::three();
    histories.catalog();
    let grow = |text: &str| {
        let record = json!({ "type": "user", "message": { "content": text } });
        append_claude(&histories, CLAUDE_RETRY, "/usr", &[record]);
    };
    grow("Grown once");
    let (mut palette, mut stepper) = histories.palette_indexing();
    keys(&mut palette, "twice");
    assert!(stepper.step());
    grow("Grown twice");
    finish(&mut stepper);
    let during = frame(&mut palette, 80, 24);
    assert!(rows_with(&during, &["Retry"]).is_empty(), "{during:#?}");

    ctrl(&mut palette, 'r');
    finish(&mut stepper);
    let after = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&after, &["Retry"]).len(), 1, "{after:#?}");
}

#[test]
fn deleting_a_sessions_folder_orphans_it_after_the_next_refresh() {
    let histories = Histories::new();
    let folder = live_dir();
    histories.claude(
        CLAUDE_NEW,
        &folder.path().to_string_lossy(),
        "Draft the guide",
    );
    histories.catalog();
    let (mut palette, mut stepper) = histories.palette_indexing();
    finish(&mut stepper);
    let before = frame(&mut palette, 80, 24);
    assert!(
        !rows_with(&before, &["Draft"])[0].contains("orphaned"),
        "{before:#?}"
    );

    drop(folder);
    ctrl(&mut palette, 'r');
    finish(&mut stepper);
    let after = frame(&mut palette, 80, 24);
    assert!(
        rows_with(&after, &["Draft"])[0].contains("orphaned"),
        "{after:#?}"
    );
}

#[test]
fn the_background_thread_waits_after_a_pass_and_runs_another_on_ctrl_r() {
    let histories = Histories::three();
    histories.catalog();
    let catalog = Catalog::cached(&histories.roots(), &histories.data()).unwrap();
    let indexer = Indexer::start(&histories.roots(), &histories.data());
    let mut palette = Palette::new(catalog, Colour::Accent, Path::new("/")).indexing(indexer);
    let deadline = Instant::now() + Duration::from_secs(10);
    // The first pass has ended once the next is due only after the interval.
    while palette
        .redraw_at()
        .is_none_or(|at| at < Instant::now() + REFRESH_INTERVAL / 2)
    {
        assert!(Instant::now() < deadline, "the first pass never ended");
        frame(&mut palette, 80, 24);
        thread::sleep(Duration::from_millis(5));
    }
    histories.claude(CLAUDE_NEW, "/usr", "Draft the onboarding guide");

    ctrl(&mut palette, 'r');
    while rows_with(&frame(&mut palette, 80, 24), &["Draft"]).is_empty() {
        assert!(Instant::now() < deadline, "Ctrl+R started no second pass");
        thread::sleep(Duration::from_millis(5));
    }
}
