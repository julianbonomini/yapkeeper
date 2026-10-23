//! PROTOTYPE, disposable. Yapkeeper's command-palette flow over your real Claude Code
//! and Codex histories (read-only), or synthetic sessions with `--demo`.
//! Resume is simulated: nothing is launched.
//!
//!   cargo run --release                 # real histories
//!   cargo run --release -- --demo       # synthetic sessions
//!   cargo run --release -- --stats      # aggregate counts only, no TUI
//!   cargo run -- --demo --dump 80x24 --keys "text:retry"   # print one frame
#![forbid(unsafe_code)]
#![allow(dead_code)]

mod app;
mod data;
mod real;
mod text;
mod ui;

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

use app::App;

struct Args {
    demo: bool,
    stats: bool,
    cwd: Option<String>,
    dump: Option<(u16, u16)>,
    keys: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        demo: false,
        stats: false,
        cwd: None,
        dump: None,
        keys: String::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--demo" => args.demo = true,
            "--stats" => args.stats = true,
            "--cwd" => args.cwd = it.next(),
            "--dump" => {
                let spec = it.next().unwrap_or_default();
                let (w, h) = spec.split_once('x').unwrap_or(("80", "24"));
                args.dump = Some((w.parse().unwrap_or(80), h.parse().unwrap_or(24)));
            }
            "--keys" => args.keys = it.next().unwrap_or_default(),
            other => {
                eprintln!(
                    "unknown argument {other}\nusage: yapkeeper-tui-prototype [--demo] [--stats] [--cwd PATH] [--dump WxH --keys \"...\"]"
                );
                std::process::exit(2);
            }
        }
    }
    args
}

/// Key script for `--dump`: `down`, `enter`, `C-x`, `F2`, `text:retry`, or a single char.
fn script_key(tok: &str) -> Vec<KeyEvent> {
    let k = |code| vec![KeyEvent::new(code, KeyModifiers::NONE)];
    match tok {
        "up" => k(KeyCode::Up),
        "down" => k(KeyCode::Down),
        "left" => k(KeyCode::Left),
        "right" => k(KeyCode::Right),
        "enter" => k(KeyCode::Enter),
        "esc" => k(KeyCode::Esc),
        "tab" => k(KeyCode::Tab),
        "space" => k(KeyCode::Char(' ')),
        "bs" => k(KeyCode::Backspace),
        "F1" => k(KeyCode::F(1)),
        "F2" => k(KeyCode::F(2)),
        t if t.starts_with("C-") => vec![KeyEvent::new(
            KeyCode::Char(t.chars().nth(2).unwrap_or(' ')),
            KeyModifiers::CONTROL,
        )],
        t if t.starts_with("text:") => t[5..]
            .chars()
            .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .collect(),
        t => t
            .chars()
            .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .collect(),
    }
}

fn dump(app: &mut App, w: u16, h: u16, keys: &str) {
    app.size = (w, h);
    for tok in keys.split_whitespace() {
        for key in script_key(tok) {
            app.on_key(key);
        }
    }
    let Ok(mut term) = Terminal::new(TestBackend::new(w, h));
    let _ = term.draw(|f| ui::draw(f, app));
    let buf = term.backend().buffer();
    for y in 0..h {
        let mut line = String::new();
        let mut skip = 0;
        for x in 0..w {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            skip = text::width(sym).saturating_sub(1);
            line.push_str(sym);
        }
        println!("{}", line.trim_end());
    }
    if let Some(r) = &app.receipt {
        println!("--- receipt ---\n{r}");
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    while !app.quit {
        let size = terminal.size()?;
        app.size = (size.width, size.height);
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        // Block on input; wake periodically only while a toast needs to expire.
        if app.toast.is_some() && !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.on_key(key);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let args = parse_args();
    let (sessions, source, default_cwd) = if args.demo {
        (
            data::sessions(),
            "synthetic".to_string(),
            data::DEFAULT_CWD.to_string(),
        )
    } else {
        eprintln!("yapkeeper prototype: reading Claude Code and Codex histories (read-only)…");
        let (sessions, st) = real::load();
        if args.stats {
            eprintln!("{st:#?}");
            return Ok(());
        }
        let here = std::env::current_dir()
            .map(|p| real::tilde(&p.to_string_lossy()))
            .unwrap_or_default();
        let source = format!(
            "your histories: {} sessions, {} folders, read in {} ms",
            st.sessions, st.locations, st.ms
        );
        (sessions, source, here)
    };
    let cwd = args.cwd.unwrap_or(default_cwd);
    let mut app = App::new(sessions, source, &cwd);
    if let Some((w, h)) = args.dump {
        dump(&mut app, w, h, &args.keys);
        return Ok(());
    }
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result?;
    if let Some(r) = app.receipt {
        print!("{r}");
    }
    Ok(())
}
