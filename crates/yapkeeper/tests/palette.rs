//! Drives `yapkeeper::palette` over synthetic Claude Code and Codex histories
//! indexed into a tempdir, and checks the frames it draws into a
//! `TestBackend` and what each key press does.

#![forbid(unsafe_code)]

use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;
use yapkeeper::palette::{Colour, Palette, Restore, Step};
use yapkeeper::{Catalog, DataDir, Roots, hide};

#[cfg(unix)]
use support::Locked;
use support::{repo_with_worktree, temp_dir};

mod support;

/// A fresh `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and data directory, each empty.
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
        }
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn catalog(&self) -> Catalog {
        Catalog::open(&self.roots(), &self.data()).unwrap()
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
}

const CLAUDE_RETRY: &str = "aaaaaaaa-0000-4000-8000-000000000401";
const CLAUDE_GONE: &str = "aaaaaaaa-0000-4000-8000-000000000402";
const CODEX: &str = "eeeeeeee-0000-7000-8000-000000000401";

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
    terminal.draw(|frame| palette.render(frame)).unwrap();
    terminal.backend().buffer().clone()
}

/// Each row of `buffer` as text, one character per glyph: the cell a wide
/// character spills into is skipped.
fn lines(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.top()..area.bottom())
        .map(|y| {
            let mut line = String::new();
            let mut skip = 0;
            for x in area.left()..area.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                skip = symbol.width().saturating_sub(1);
                line.push_str(symbol);
            }
            line
        })
        .collect()
}

fn frame(palette: &mut Palette, width: u16, height: u16) -> Vec<String> {
    lines(&draw(palette, width, height))
}

fn press(palette: &mut Palette, code: KeyCode) -> Step {
    palette.handle(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(palette: &mut Palette, c: char) -> Step {
    palette.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
        assert!(row.contains('/'), "location: {row}");
        assert!(ends_with_age(row), "age: {row}");
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    let frame = frame(&mut palette, 80, 24);
    assert!(frame[1].starts_with("│ › "), "{frame:#?}");
    assert!(
        frame[1].trim_end_matches('│').trim_end().ends_with("3 / 3"),
        "{frame:#?}"
    );
    assert!(frame[22].contains("↑↓ move"), "{frame:#?}");
    let rows = rows_with(&frame, &TITLES);
    assert_eq!(rows.len(), 3, "{frame:#?}");
    // The inner width is 78; the results keep 48% of it.
    let preview_from = 1 + (78 * 48_usize).div_ceil(100);
    for row in rows {
        let preview: String = row
            .chars()
            .skip(preview_from)
            .take(78 - preview_from + 1)
            .collect();
        assert!(preview.trim().is_empty(), "{row}");
        assert!(ends_with_age(row), "{row}");
    }
}

#[test]
fn a_narrow_terminal_gives_results_the_full_width() {
    let histories = Histories::three();
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();

    let mut palette = Palette::new(&catalog, Colour::Sage);
    keys(&mut palette, "#codex");
    let codex = frame(&mut palette, 120, 40);
    assert_eq!(rows_with(&codex, &TITLES).len(), 1, "{codex:#?}");
    assert_eq!(rows_with(&codex, &["Tidy"]).len(), 1);
    assert!(codex[1].trim_end_matches('│').trim_end().ends_with("1 / 3"));

    let mut palette = Palette::new(&catalog, Colour::Sage);
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    keys(&mut palette, "release");
    assert!(frame(&mut palette, 80, 24)[22].contains("Esc clear"));
    assert_eq!(press(&mut palette, KeyCode::Esc), Step::Continue);
    let cleared = frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&cleared, &TITLES).len(), 3, "{cleared:#?}");
    assert!(cleared[22].contains("Esc quit"), "{cleared:#?}");
    assert_eq!(press(&mut palette, KeyCode::Esc), Step::Quit);

    let mut palette = Palette::new(&catalog, Colour::Sage);
    keys(&mut palette, "release");
    assert_eq!(ctrl(&mut palette, 'u'), Step::Continue);
    assert_eq!(rows_with(&frame(&mut palette, 80, 24), &TITLES).len(), 3);
    keys(&mut palette, "release");
    assert_eq!(ctrl(&mut palette, 'c'), Step::Quit);
}

#[test]
fn sage_draws_the_selection_bar_grey_and_the_prompt_in_the_accent() {
    let histories = Histories::three();
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Mono);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    keys(&mut palette, "zebra");
    let frame = frame(&mut palette, 80, 24);
    assert_eq!(frame[3].trim_matches('│').trim(), "No sessions match.");
    assert_eq!(
        frame[4].trim_matches('│').trim(),
        "is:temp shows temp sessions"
    );
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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

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
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    keys(&mut palette, "retry");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| palette.render(frame)).unwrap();
    let after_word = terminal.get_cursor_position().unwrap();
    keys(&mut palette, " ");
    terminal.draw(|frame| palette.render(frame)).unwrap();
    let after_space = terminal.get_cursor_position().unwrap();
    assert_eq!(after_space.x, after_word.x + 1);
    assert_eq!(after_space.y, 1);
}

#[test]
fn an_empty_index_says_no_sessions_were_found() {
    let histories = Histories::new();
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    let frame = frame(&mut palette, 80, 24);
    assert_eq!(frame[3].trim_matches('│').trim(), "No sessions found.");
    assert!(frame[1].trim_end_matches('│').trim_end().ends_with("0 / 0"));
}

#[test]
fn a_query_that_matches_only_hidden_sessions_says_how_to_show_them() {
    let histories = Histories::three();
    hide(&histories.roots(), &histories.data(), CLAUDE_GONE).unwrap();
    let catalog = histories.catalog();
    let mut palette = Palette::new(&catalog, Colour::Sage);

    assert_eq!(rows_with(&frame(&mut palette, 80, 24), &TITLES).len(), 2);
    keys(&mut palette, "release");
    let frame = frame(&mut palette, 80, 24);
    assert_eq!(frame[3].trim_matches('│').trim(), "No sessions match.");
    assert_eq!(
        frame[4].trim_matches('│').trim(),
        "is:hidden shows hidden ones"
    );
    keys(&mut palette, " is:hidden");
    let shown = self::frame(&mut palette, 80, 24);
    assert_eq!(rows_with(&shown, &["Release"]).len(), 1, "{shown:#?}");
}
