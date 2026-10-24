//! Running a refresh of the index in the background: the thread it runs
//! on, never joined, the flag that stops it, the latest-wins hand-over of
//! what each committed batch left in the index, with locations read off the
//! palette's thread, and that a busy index ends a pass quietly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use yapkeeper_model::Adapter;

use crate::index::Pass;
use crate::{DataDir, DataError, Roots, Snapshot, adapters};

/// Brings the index up to date in the background, one small committed batch
/// at a time, so a palette given it shows each batch's sessions as they land.
pub struct Indexer {
    shared: Shared,
}

/// What the pass is doing, as the palette shows it.
#[derive(Clone)]
pub(crate) enum Indexing {
    /// A pass is running, from `start` or `stepped` until it ends.
    Running,
    Idle,
    /// The last pass stopped on this error, for the footer.
    Failed(String),
}

impl Indexer {
    /// Starts one pass over every changed history file now, on a thread of
    /// its own, and returns at once. Failures are shown by the palette given it.
    pub fn start(roots: &Roots, data: &DataDir) -> Indexer {
        let shared = Shared::new(Indexing::Running);
        let worker = shared.clone();
        let roots = roots.clone();
        let path = data.index();
        // Never joined: quitting and resuming don't wait for it, and the
        // index stays as of its last commit when the process ends.
        thread::spawn(move || {
            // Built here, as adapters can't be sent between threads.
            let adapters = adapters(&roots);
            let adapters: Vec<&dyn Adapter> = adapters.iter().map(Box::as_ref).collect();
            let ended = Pass::new(&adapters, &path).and_then(|mut pass| {
                while worker.batch(&mut pass, &adapters)? {}
                Ok(())
            });
            worker.end(ended);
        });
        Indexer { shared }
    }

    /// The same pass, run only as the returned `Stepper` steps it, on the
    /// stepping thread.
    pub fn stepped(roots: &Roots, data: &DataDir) -> (Indexer, Stepper) {
        let shared = Shared::new(Indexing::Running);
        let stepper = Stepper {
            adapters: adapters(roots).into(),
            path: data.index(),
            stage: Stage::Unplanned,
            shared: shared.clone(),
        };
        (Indexer { shared }, stepper)
    }

    /// Never indexes; what `Palette::new` holds.
    pub(crate) fn idle() -> Indexer {
        Indexer {
            shared: Shared::new(Indexing::Idle),
        }
    }

    /// The newest snapshot published since the last call; older ones are
    /// dropped unseen.
    pub(crate) fn take(&mut self) -> Option<Snapshot> {
        self.shared.slot().latest.take()
    }

    /// What the pass is doing now. A failure is said by one call only; the
    /// indexer is idle after it.
    pub(crate) fn status(&mut self) -> Indexing {
        let mut slot = self.shared.slot();
        match slot.status {
            Indexing::Failed(_) => std::mem::replace(&mut slot.status, Indexing::Idle),
            _ => slot.status.clone(),
        }
    }
}

/// The stepping side of `Indexer::stepped`.
pub struct Stepper {
    adapters: Vec<Box<dyn Adapter>>,
    path: PathBuf,
    stage: Stage,
    shared: Shared,
}

/// How far a stepped pass has got.
enum Stage {
    /// Not yet stepped, so nothing is planned.
    Unplanned,
    Batching(Box<Pass>),
    Ended,
}

impl Stepper {
    /// Commits the pass's next batch and publishes it to the indexer. False
    /// once the pass has finished, or has failed, and the indexer is idle.
    pub fn step(&mut self) -> bool {
        let adapters: Vec<&dyn Adapter> = self.adapters.iter().map(Box::as_ref).collect();
        if let Stage::Unplanned = self.stage {
            match Pass::new(&adapters, &self.path) {
                Ok(pass) => self.stage = Stage::Batching(Box::new(pass)),
                Err(error) => {
                    self.stage = Stage::Ended;
                    self.shared.end(Err(error));
                }
            }
        }
        let Stage::Batching(pass) = &mut self.stage else {
            return false;
        };
        match self.shared.batch(pass, &adapters) {
            Ok(true) => true,
            ended => {
                self.stage = Stage::Ended;
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
    /// Set when the indexer is dropped; the pass stops before its next file.
    stop: Arc<AtomicBool>,
}

/// What a pass has published and not yet been taken.
struct Slot {
    latest: Option<Snapshot>,
    status: Indexing,
}

impl Shared {
    fn new(status: Indexing) -> Shared {
        Shared {
            slot: Arc::new(Mutex::new(Slot {
                latest: None,
                status,
            })),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        // A panic while holding the lock leaves a slot that is still whole.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Commits `pass`'s next batch and publishes the index as it left it.
    /// False, publishing nothing, once the pass has nothing left or is stopped.
    fn batch(&self, pass: &mut Pass, adapters: &[&dyn Adapter]) -> Result<bool, DataError> {
        if !pass.next_batch(adapters, &self.stop)? {
            return Ok(false);
        }
        let snapshot = Snapshot::of(pass.sessions()?);
        self.slot().latest = Some(snapshot);
        Ok(true)
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
    /// Asks the pass to stop and returns without waiting for it. A batch not
    /// yet committed is discarded, so the index stays as of the last commit.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}
