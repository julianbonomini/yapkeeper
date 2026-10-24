//! Measures how fast Yapkeeper opens and searches, on a synthetic corpus
//! written from a seed or, when asked, on the histories this environment
//! names. `corpus` owns the synthetic histories and the queries that search
//! them; `measure` owns the timing boundaries and the percentiles.

#![forbid(unsafe_code)]

mod corpus;
mod measure;

pub use corpus::{Corpus, Shape, generate};
pub use measure::{FRAME, Latency, Measurements, Paste, SlowestKeystroke, measure};
