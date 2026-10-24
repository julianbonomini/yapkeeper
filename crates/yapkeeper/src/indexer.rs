//! Refreshing the index in the background while the palette is open: when
//! each pass starts (at once, `REFRESH_INTERVAL` after the last one ended,
//! or when asked), the thread they run on, never joined, the flag that stops
//! it, the latest-wins hand-over of what each committed batch left in the
//! index, with locations read off the palette's thread, and that a busy
//! index ends a pass quietly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use yapkeeper_model::Adapter;

use crate::index::Pass;
use crate::{DataDir, DataError, Roots, Snapshot, adapters};

/// How long after a pass ends the next one starts while the palette is open.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Brings the index up to date in the background, one small committed batch
/// at a time, so a palette given it shows each batch's sessions as they land,
/// and does so again `REFRESH_INTERVAL` after each pass, or on `refresh`.
pub struct Indexer {
    /// `None` for `idle`, which never indexes.
    shared: Option<Shared>,
    /// When `status` first found no pass running.
    idle_since: Option<Instant>,
}

/// What the pass is doing, as the palette shows it.
#[derive(Clone)]
pub(crate) enum Indexing {
    /// A pass is running, or asked for, until it ends.
    Running,
    Idle,
    /// The last pass stopped on this error, for the footer.
    Failed(String),
}

impl Indexer {
    /// Starts one pass over every changed history file now, on a thread of
    /// its own, and returns at once; later passes run on the same thread.
    /// Failures are shown by the palette given it.
    pub fn start(roots: &Roots, data: &DataDir) -> Indexer {
        let shared = Shared::new();
        let worker = shared.clone();
        let roots = roots.clone();
        let path = data.index();
        // Never joined: quitting and resuming don't wait for it, and the
        // index stays as of its last commit when the process ends.
        thread::spawn(move || {
            // Built here, as adapters can't be sent between threads.
            let mut stepper = Stepper::new(&roots, path, worker);
            while stepper.shared.wait_for_pass() {
                while stepper.step() {}
            }
        });
        Indexer::over(shared)
    }

    /// The same passes, each run only as the returned `Stepper` steps it,
    /// on the stepping thread.
    pub fn stepped(roots: &Roots, data: &DataDir) -> (Indexer, Stepper) {
        let shared = Shared::new();
        let stepper = Stepper::new(roots, data.index(), shared.clone());
        (Indexer::over(shared), stepper)
    }

    fn over(shared: Shared) -> Indexer {
        Indexer {
            shared: Some(shared),
            idle_since: None,
        }
    }

    /// Never indexes; what `Palette::new` holds.
    pub(crate) fn idle() -> Indexer {
        Indexer {
            shared: None,
            idle_since: None,
        }
    }

    /// The newest snapshot published since the last call; older ones are
    /// dropped unseen.
    pub(crate) fn take(&mut self) -> Option<Snapshot> {
        self.shared.as_ref()?.slot().latest.take()
    }

    /// What the pass is doing at `now`, having first started the next one
    /// when `REFRESH_INTERVAL` has passed since a call found none running. A
    /// failure is said by one call only; the indexer is idle after it.
    pub(crate) fn status(&mut self, now: Instant) -> Indexing {
        let Some(shared) = &self.shared else {
            return Indexing::Idle;
        };
        let mut slot = shared.slot();
        if let Indexing::Running = slot.status {
            return Indexing::Running;
        }
        let since = *self.idle_since.get_or_insert(now);
        if now < since + REFRESH_INTERVAL {
            return std::mem::replace(&mut slot.status, Indexing::Idle);
        }
        drop(slot);
        self.refresh();
        Indexing::Running
    }

    /// When `status` will next start a pass; `None` while one runs, or for
    /// `idle`.
    pub(crate) fn next_pass_at(&self) -> Option<Instant> {
        self.idle_since.map(|since| since + REFRESH_INTERVAL)
    }

    /// Starts a pass now, unless one is already running.
    pub(crate) fn refresh(&mut self) {
        self.idle_since = None;
        if let Some(shared) = &self.shared {
            shared.slot().status = Indexing::Running;
            shared.wake.notify_all();
        }
    }
}

/// The stepping side of `Indexer::stepped`.
pub struct Stepper {
    adapters: Vec<Box<dyn Adapter>>,
    path: PathBuf,
    /// The pass being run, once planned by its first step.
    pass: Option<Box<Pass>>,
    shared: Shared,
}

impl Stepper {
    fn new(roots: &Roots, path: PathBuf, shared: Shared) -> Stepper {
        Stepper {
            adapters: adapters(roots).into(),
            path,
            pass: None,
            shared,
        }
    }

    /// Commits the next batch of the pass the indexer asked for, and
    /// publishes it to the indexer. False when no pass is asked for, and
    /// once the pass has finished, or has failed, and the indexer is idle.
    pub fn step(&mut self) -> bool {
        if !self.shared.running() {
            return false;
        }
        let adapters: Vec<&dyn Adapter> = self.adapters.iter().map(Box::as_ref).collect();
        let pass = match &mut self.pass {
            Some(pass) => pass,
            None => match Pass::new(&adapters, &self.path) {
                Ok(pass) => self.pass.insert(Box::new(pass)),
                Err(error) => {
                    self.shared.end(Err(error));
                    return false;
                }
            },
        };
        match self.shared.batch(pass, &adapters) {
            Ok(true) => true,
            ended => {
                self.pass = None;
                self.shared.end(ended.map(|_| ()));
                false
            }
        }
    }
}

/// What a pass and the indexer watching it share.
#[derive(Clone)]
struct Shared {
    slot: Arc<Mutex<Slot>>,
    /// Wakes the thread waiting for the next pass.
    wake: Arc<Condvar>,
    /// Set when the indexer is dropped; the pass stops before its next file.
    stop: Arc<AtomicBool>,
}

/// What a pass has published and not yet been taken.
struct Slot {
    latest: Option<Snapshot>,
    status: Indexing,
}

impl Shared {
    /// Asking for a pass at once.
    fn new() -> Shared {
        Shared {
            slot: Arc::new(Mutex::new(Slot {
                latest: None,
                status: Indexing::Running,
            })),
            wake: Arc::new(Condvar::new()),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        // A panic while holding the lock leaves a slot that is still whole.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn running(&self) -> bool {
        matches!(self.slot().status, Indexing::Running)
    }

    /// Waits until a pass is asked for, giving true, or the indexer is
    /// dropped, giving false.
    fn wait_for_pass(&self) -> bool {
        let stopped = || self.stop.load(Ordering::Relaxed);
        let waited = self.wake.wait_while(self.slot(), |slot| {
            !matches!(slot.status, Indexing::Running) && !stopped()
        });
        drop(waited.unwrap_or_else(PoisonError::into_inner));
        !stopped()
    }

    /// Commits `pass`'s next batch and publishes the index as it left it.
    /// Once nothing is left it publishes once more, so every pass reads
    /// each session's location again even when no file changed, and gives
    /// false. Publishes nothing once stopped.
    fn batch(&self, pass: &mut Pass, adapters: &[&dyn Adapter]) -> Result<bool, DataError> {
        let committed = pass.next_batch(adapters, &self.stop)?;
        if self.stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let snapshot = Snapshot::of(pass.sessions()?);
        self.slot().latest = Some(snapshot);
        Ok(committed)
    }

    /// Records how the pass ended. An index another process kept busy ends
    /// it quietly, as the next pass reads what this one left.
    fn end(&self, ended: Result<(), DataError>) {
        self.slot().status = match ended {
            Ok(()) => Indexing::Idle,
            Err(error) if error.is_busy() => Indexing::Idle,
            Err(error) => Indexing::Failed(error.to_string()),
        };
    }
}

impl Drop for Indexer {
    /// Asks the pass to stop, or the thread waiting for the next one to
    /// end, and returns without waiting for either. A batch not yet
    /// committed is discarded, so the index stays as of the last commit.
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.stop.store(true, Ordering::Relaxed);
            // Under the lock, so a thread about to wait can't miss it.
            let _slot = shared.slot();
            shared.wake.notify_all();
        }
    }
}
