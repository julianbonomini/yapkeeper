//! The palette's hold on the real terminal: raw mode and the alternate
//! screen while it runs, and putting both back on every way out, a panic
//! included, without leaving its panic hook behind; and the event loop,
//! which reads from an `Events` source so tests can script it.

use std::io::{self, Write, stdout};
use std::panic;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use super::{Palette, Step};
use crate::ResumeCommand;

/// Takes over the terminal (raw mode, alternate screen), runs `palette`
/// until it quits, and restores the terminal on return, on error and on
/// panic, and returns the command the person confirmed, if any. By the
/// time it returns, the palette and its catalog are dropped, raw mode is
/// off, the alternate screen is gone, the cursor shows and the previous
/// panic hook is back. Fails when the terminal can't be set up, drawn to
/// or read from.
pub fn run(mut palette: Palette) -> io::Result<Option<ResumeCommand>> {
    enable_raw_mode()?;
    // Held before anything else changes, so every later failure restores.
    let _restore = Restore::new(stdout());
    execute!(stdout(), EnterAlternateScreen)?;
    // The hook restores before the previous one prints, so the panic
    // message lands on the main screen in cooked mode.
    let previous = Arc::new(panic::take_hook());
    let chained = Arc::clone(&previous);
    panic::set_hook(Box::new(move |info| {
        restore(&mut stdout());
        chained(info);
    }));
    let result = Terminal::new(CrosstermBackend::new(stdout()))
        .and_then(|mut terminal| event_loop(&mut palette, &mut terminal, &mut Crossterm));
    // Dropping ours releases its share, so the previous hook can go back.
    drop(panic::take_hook());
    if let Ok(previous) = Arc::try_unwrap(previous) {
        panic::set_hook(previous);
    }
    result
}

/// Where the palette's events come from: the terminal, or a script in tests.
pub trait Events {
    /// The next event, if one comes within `timeout`. `None` waits as long
    /// as it takes; `Some(Duration::ZERO)` only takes an event already
    /// waiting. Fails when the source can't be read.
    fn next_event(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>>;
}

/// The real terminal's events, through crossterm.
struct Crossterm;

impl Events for Crossterm {
    fn next_event(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>> {
        if let Some(timeout) = timeout
            && !event::poll(timeout)?
        {
            return Ok(None);
        }
        event::read().map(Some)
    }
}

/// Draws `palette` into `terminal` and applies what `events` gives until the
/// person quits (`None`) or confirms resuming (the command). After each
/// event it applies every event already waiting, then draws once; keys
/// after one that quits or resumes are not applied. With nothing waiting it
/// redraws at `palette.redraw_at()`. Leaves the terminal's modes alone;
/// `run` owns them. Fails when `terminal` can't be drawn to or `events`
/// can't be read.
pub fn event_loop<B: Backend<Error = io::Error>>(
    palette: &mut Palette,
    terminal: &mut Terminal<B>,
    events: &mut impl Events,
) -> io::Result<Option<ResumeCommand>> {
    loop {
        terminal.draw(|frame| palette.render(frame, Instant::now()))?;
        // With nothing pressed by then, redraw when the frame changes.
        let timeout = palette
            .redraw_at()
            .map(|at| at.saturating_duration_since(Instant::now()));
        let mut waiting = events.next_event(timeout)?;
        // Everything already waiting, a paste say, is applied before one draw.
        while let Some(event) = waiting {
            // A resize, or any other event, just redraws.
            if let Event::Key(key) = event {
                match palette.handle(key, Instant::now()) {
                    Step::Continue => {}
                    Step::Quit => return Ok(None),
                    Step::Resume(command) => return Ok(Some(command)),
                }
            }
            waiting = events.next_event(Some(Duration::ZERO))?;
        }
    }
}

/// The guard `run` holds: when dropped, including while a panic unwinds,
/// it turns raw mode off, leaves the alternate screen and shows the cursor,
/// writing the escape codes to `out`. Never fails.
pub struct Restore<W: Write> {
    out: W,
}

impl<W: Write> Restore<W> {
    /// A guard that restores the terminal through `out` when dropped.
    pub fn new(out: W) -> Restore<W> {
        Restore { out }
    }
}

impl<W: Write> Drop for Restore<W> {
    fn drop(&mut self) {
        restore(&mut self.out);
    }
}

/// Puts the terminal back as the shell left it. Each step is tried even
/// when one before it failed, since there is nowhere left to report to.
fn restore(out: &mut impl Write) {
    let _ = disable_raw_mode();
    let _ = execute!(out, LeaveAlternateScreen, Show);
    let _ = out.flush();
}
