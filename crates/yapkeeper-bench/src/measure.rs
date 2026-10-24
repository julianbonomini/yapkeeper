//! Times the palette as a person meets it: opening it over a warm index,
//! up to its first frame, and typing a query a key at a time and pasting
//! one, each up to the frame showing the query's results, which includes
//! the search worker's hand-over and the palette's redraw tick. Frames are
//! drawn off-screen. The timing boundaries and the percentiles live here.

use std::num::NonZeroUsize;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yapkeeper::palette::{Colour, Palette};
use yapkeeper::{Catalog, DataDir, DataError, Roots, list};

use crate::corpus::{PASTED, QUERIES};

/// The size every frame is drawn at, wide enough for the preview.
pub const FRAME: (u16, u16) = (120, 40);

/// The median and the 95th percentile of one measurement's runs.
#[derive(Debug, Clone, Copy)]
pub struct Latency {
    pub p50: Duration,
    pub p95: Duration,
}

/// The slowest keystroke of typing `query` from an empty query: the one
/// that completed `prefix`, timed from its key press to the frame showing
/// that prefix's results, which can come a frame or more after the key.
pub struct SlowestKeystroke {
    pub query: &'static str,
    pub prefix: &'static str,
    pub latency: Latency,
}

/// Every key of `query` pressed with no frame drawn between them, timed
/// from the first key press to the frame showing the query's results.
pub struct Paste {
    pub query: &'static str,
    pub latency: Latency,
}

/// What `measure` timed.
pub struct Measurements {
    /// Sessions in the index once it was warmed.
    pub indexed: usize,
    /// How long building the index from nothing took.
    pub warmed: Duration,
    /// From opening the catalog over the warm index to the palette's first
    /// frame drawn.
    pub launch: Latency,
    /// One per representative query, in the order they were typed.
    pub slowest_keystrokes: Vec<SlowestKeystroke>,
    /// The representative query pasted.
    pub paste: Paste,
}

/// Builds the index in `data` from `roots`, then opens the palette over it
/// `runs` times, types each representative query a key at a time `runs`
/// times and pastes one `runs` times, each from an empty query. Launch is
/// timed up to the first frame, typing and pasting up to the frame showing
/// the query's results, each drawn at `FRAME` off-screen. Process start, the
/// terminal's own drawing and the background indexer are outside every
/// timing. Reads the histories only to build the index, and
/// writes only in `data`. Fails when the index or the preferences can't be
/// used.
pub fn measure(
    roots: &Roots,
    data: &DataDir,
    runs: NonZeroUsize,
) -> Result<Measurements, DataError> {
    let started = Instant::now();
    let indexed = list(roots, data)?.indexed;
    let warmed = started.elapsed();

    let mut terminal = Terminal::new(TestBackend::new(FRAME.0, FRAME.1))
        .expect("an off-screen terminal never fails");
    let mut draw = |palette: &mut Palette| {
        terminal
            .draw(|frame| palette.render(frame, Instant::now()))
            .expect("an off-screen terminal never fails");
    };

    let mut launches = Vec::with_capacity(runs.get());
    let mut palette = None;
    for _ in 0..runs.get() {
        let started = Instant::now();
        let catalog = Catalog::cached(roots, data)?.searching_in_background();
        let mut opened = Palette::new(catalog, Colour::Accent, Path::new("/"));
        draw(&mut opened);
        launches.push(started.elapsed());
        palette = Some(opened);
    }
    let mut palette = palette.expect("runs is never 0");

    let slowest_keystrokes = QUERIES
        .into_iter()
        .map(|query| {
            // Each prefix's keystroke times, across runs.
            let mut times = vec![Vec::with_capacity(runs.get()); query.chars().count()];
            for _ in 0..runs.get() {
                clear(&mut palette);
                draw_results(&mut palette, &mut draw);
                for (c, prefix_times) in query.chars().zip(&mut times) {
                    let started = Instant::now();
                    press(&mut palette, character(c));
                    draw_results(&mut palette, &mut draw);
                    prefix_times.push(started.elapsed());
                }
            }
            let (prefix, latency) = query
                .char_indices()
                .map(|(at, c)| &query[..at + c.len_utf8()])
                .zip(times.into_iter().map(Latency::of))
                .max_by_key(|(_, latency)| latency.p95)
                .expect("no query is empty");
            SlowestKeystroke {
                query,
                prefix,
                latency,
            }
        })
        .collect();

    let pastes = (0..runs.get())
        .map(|_| {
            clear(&mut palette);
            draw_results(&mut palette, &mut draw);
            let started = Instant::now();
            for c in PASTED.chars() {
                press(&mut palette, character(c));
            }
            draw_results(&mut palette, &mut draw);
            started.elapsed()
        })
        .collect();

    Ok(Measurements {
        indexed,
        warmed,
        launch: Latency::of(launches),
        slowest_keystrokes,
        paste: Paste {
            query: PASTED,
            latency: Latency::of(pastes),
        },
    })
}

/// Draws `palette` with `draw`, then again at each redraw it asks for, as
/// the terminal's loop does, until the frame drawn shows the query's results.
fn draw_results(palette: &mut Palette, draw: &mut impl FnMut(&mut Palette)) {
    draw(palette);
    while !palette.caught_up() {
        if let Some(at) = palette.redraw_at() {
            thread::sleep(at.saturating_duration_since(Instant::now()));
        }
        draw(palette);
    }
}

/// Empties the query, as `Ctrl+U` does, so each run starts afresh.
fn clear(palette: &mut Palette) {
    press(
        palette,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
}

fn press(palette: &mut Palette, key: KeyEvent) {
    palette.handle(key, Instant::now());
}

fn character(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

impl Latency {
    /// The nearest-rank percentiles of `times`, which is never empty.
    fn of(mut times: Vec<Duration>) -> Latency {
        times.sort();
        let rank = |percent: usize| times[(times.len() * percent).div_ceil(100) - 1];
        Latency {
            p50: rank(50),
            p95: rank(95),
        }
    }
}
