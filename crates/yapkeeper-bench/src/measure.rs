//! Times the palette as a person meets it: opening it over a warm index
//! and typing a query, each up to a frame drawn off-screen. The timing
//! boundaries and the percentiles live here.

use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yapkeeper::palette::{Colour, Palette};
use yapkeeper::{Catalog, DataDir, DataError, Roots, list};

use crate::corpus::QUERIES;

/// The size every frame is drawn at, wide enough for the preview.
pub const FRAME: (u16, u16) = (120, 40);

/// The median and the 95th percentile of one measurement's runs.
#[derive(Debug, Clone, Copy)]
pub struct Latency {
    pub p50: Duration,
    pub p95: Duration,
}

/// The time to type the last character of `query` and see the frame.
pub struct Search {
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
    pub searches: Vec<Search>,
}

/// Builds the index in `data` from `roots`, then opens the palette over it
/// `runs` times and types each representative query `runs` times, timing
/// each up to a frame drawn at `FRAME` off-screen. Process start, the
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
        let catalog = Catalog::cached(roots, data)?;
        let mut opened = Palette::new(catalog, Colour::Accent, Path::new("/"));
        draw(&mut opened);
        launches.push(started.elapsed());
        palette = Some(opened);
    }
    let mut palette = palette.expect("runs is never 0");

    let searches = QUERIES
        .into_iter()
        .map(|query| {
            let last = query.chars().next_back().expect("no query is empty");
            let typed = &query[..query.len() - last.len_utf8()];
            let times = (0..runs.get())
                .map(|_| {
                    press(
                        &mut palette,
                        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                    );
                    for c in typed.chars() {
                        press(&mut palette, character(c));
                    }
                    draw(&mut palette);
                    let started = Instant::now();
                    press(&mut palette, character(last));
                    draw(&mut palette);
                    started.elapsed()
                })
                .collect();
            Search {
                query,
                latency: Latency::of(times),
            }
        })
        .collect();

    Ok(Measurements {
        indexed,
        warmed,
        launch: Latency::of(launches),
        searches,
    })
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
