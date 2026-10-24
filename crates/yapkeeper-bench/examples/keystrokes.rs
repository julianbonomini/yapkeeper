// TEMPORARY investigation probe: times every keystroke of a "paste".
#![forbid(unsafe_code)]
use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yapkeeper::palette::{Colour, Palette};
use yapkeeper::{Catalog, DataDir, list};
use yapkeeper_bench::{Shape, generate};

fn ms(d: Duration) -> String { format!("{:7.1}", d.as_secs_f64() * 1000.0) }

fn main() {
    let mb: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(300);
    let paste = std::env::args().nth(2).unwrap_or("database migration retry".into());
    let corpus_dir = tempfile::Builder::new().prefix(".yk-probe-").tempdir_in(".").unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(index_dir.path().to_path_buf());
    let shape = Shape { bytes: mb * 1024 * 1024, ..Shape::default() };
    let corpus = generate(&shape, corpus_dir.path()).unwrap();
    let t = Instant::now();
    let n = list(&corpus.roots, &data).unwrap().indexed;
    eprintln!("indexed {n} sessions ({mb} MiB) in {:?}", t.elapsed());

    // Per-stage costs of the catalog calls one keystroke makes.
    let catalog = Catalog::cached(&corpus.roots, &data).unwrap();
    println!("prefix                          parse  search compl  convo   (ms)");
    for i in 1..=paste.len() {
        let text = &paste[..i];
        let t = Instant::now(); let q = catalog.parse(text); let p = t.elapsed();
        let t = Instant::now(); let found = catalog.search(&q).unwrap(); let s = t.elapsed();
        let t = Instant::now(); let _ = catalog.completions(text); let c = t.elapsed();
        let t = Instant::now();
        if let Some(first) = found.sessions.first() { let _ = catalog.conversation(&first.session).unwrap(); }
        let v = t.elapsed();
        println!("{:<30}{}{}{}{}  hits={}", format!("{text:?}"), ms(p), ms(s), ms(c), ms(v), found.sessions.len());
    }

    // The whole palette, as the event loop runs it: handle then draw, per char.
    let catalog = Catalog::cached(&corpus.roots, &data).unwrap();
    let mut palette = Palette::new(catalog, Colour::Accent, Path::new("/"));
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
    term.draw(|f| palette.render(f, Instant::now())).unwrap();
    let start = Instant::now();
    let (mut th, mut td) = (Duration::ZERO, Duration::ZERO);
    for c in paste.chars() {
        let t = Instant::now();
        palette.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), Instant::now());
        th += t.elapsed();
        let t = Instant::now();
        term.draw(|f| palette.render(f, Instant::now())).unwrap();
        td += t.elapsed();
    }
    println!("\npaste of {} chars through the palette: total {} ms (handle {} ms, draw {} ms)",
        paste.len(), ms(start.elapsed()), ms(th), ms(td));
}
