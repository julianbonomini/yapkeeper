//! The palette's hold on the real terminal: raw mode and the alternate
//! screen while it runs, and putting both back on every way out, a panic
//! included, without leaving its panic hook behind.

use std::io::{self, Write, stdout};
use std::panic;
use std::sync::Arc;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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
    let result = event_loop(&mut palette);
    // Dropping ours releases its share, so the previous hook can go back.
    drop(panic::take_hook());
    if let Ok(previous) = Arc::try_unwrap(previous) {
        panic::set_hook(previous);
    }
    result
}

/// Draws and reads keys until the person quits, giving `None`, or
/// confirms resuming, giving the command.
fn event_loop(palette: &mut Palette) -> io::Result<Option<ResumeCommand>> {
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    loop {
        terminal.draw(|frame| palette.render(frame, Instant::now()))?;
        // With nothing pressed by then, redraw when the frame changes.
        if let Some(at) = palette.redraw_at()
            && !event::poll(at.saturating_duration_since(Instant::now()))?
        {
            continue;
        }
        // A resize, or any other event, just redraws.
        if let Event::Key(key) = event::read()? {
            match palette.handle(key, Instant::now()) {
                Step::Continue => {}
                Step::Quit => return Ok(None),
                Step::Resume(command) => return Ok(Some(command)),
            }
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
