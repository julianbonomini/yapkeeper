//! Finding which sessions a query's words match off the palette's thread:
//! the thread, never joined, its own read-only connection, the latest-wins
//! hand-over each way (an unstarted ask is replaced; only the newest answer
//! for the query last asked is handed over), and narrowing from the last
//! lookup.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::index::Index;
use crate::{DataError, Query, SessionKey};

/// Finds which sessions a query's words match on a worker, handing back
/// only the newest.
pub(crate) struct Searcher {
    shared: Shared,
    /// The sequence number of the last ask.
    asked: u64,
    last_query: Option<Query>,
    /// The sequence number of the last answer handed over.
    handed: u64,
}

/// The sessions a query's words matched, before its filters and defaults
/// were applied.
pub(crate) struct Matched {
    pub(crate) query: Query,
    pub(crate) keys: Result<HashSet<SessionKey>, DataError>,
}

impl Searcher {
    /// Searches on a thread of its own, which reads the index at `index`
    /// with a connection of its own. Returns at once.
    pub(crate) fn start(index: PathBuf) -> Searcher {
        let shared = Shared::new();
        let mut stepper = SearchStepper::new(index, shared.clone());
        // Never joined: quitting and resuming don't wait for a search, and a
        // search only reads.
        thread::spawn(move || {
            while stepper.shared.wait_for_request() {
                stepper.step();
            }
        });
        Searcher::over(shared)
    }

    /// The same searches, each run only as the returned `SearchStepper`
    /// steps, on the stepping thread.
    pub(crate) fn stepped(index: PathBuf) -> (Searcher, SearchStepper) {
        let shared = Shared::new();
        let stepper = SearchStepper::new(index, shared.clone());
        (Searcher::over(shared), stepper)
    }

    fn over(shared: Shared) -> Searcher {
        Searcher {
            shared,
            asked: 0,
            last_query: None,
            handed: 0,
        }
    }

    /// Asks which of `among` match `query`'s words, dropping an earlier ask
    /// not yet started. A query with no words is answered at once, without
    /// the index, but still through `take`.
    pub(crate) fn ask(&mut self, query: Query, among: Arc<HashSet<SessionKey>>) {
        self.asked += 1;
        self.last_query = Some(query.clone());
        let mut slot = self.shared.slot();
        if query.has_words() {
            slot.request = Some(Request {
                seq: self.asked,
                query,
                among,
            });
            self.shared.wake.notify_all();
        } else {
            slot.request = None;
            slot.answer(Answer {
                seq: self.asked,
                matched: Matched {
                    query,
                    keys: Ok((*among).clone()),
                },
            });
        }
    }

    /// The newest answer for the query last asked, not taken before.
    pub(crate) fn take(&mut self) -> Option<Matched> {
        let answer = self.shared.slot().answer.take()?;
        if Some(&answer.matched.query) != self.last_query.as_ref() {
            return None;
        }
        self.handed = answer.seq;
        Some(answer.matched)
    }

    /// Whether the answer to the last ask is still to come.
    pub(crate) fn searching(&self) -> bool {
        self.handed < self.asked
    }
}

/// The stepping side of `Catalog::searching_stepped`.
pub struct SearchStepper {
    shared: Shared,
    path: PathBuf,
    /// Opened by the first search; a failed open is tried again by the next.
    index: Option<Index>,
    matcher: Matcher,
}

impl SearchStepper {
    fn new(path: PathBuf, shared: Shared) -> SearchStepper {
        SearchStepper {
            shared,
            path,
            index: None,
            matcher: Matcher::default(),
        }
    }

    /// Runs the newest search asked for and not yet started, and hands its
    /// results to the catalog. False when none is waiting.
    pub fn step(&mut self) -> bool {
        let Some(request) = self.shared.slot().request.take() else {
            return false;
        };
        let keys = self.matching(&request);
        self.shared.slot().answer(Answer {
            seq: request.seq,
            matched: Matched {
                query: request.query,
                keys,
            },
        });
        true
    }

    fn matching(&mut self, request: &Request) -> Result<HashSet<SessionKey>, DataError> {
        let index = match &mut self.index {
            Some(index) => index,
            None => self.index.insert(Index::reading(&self.path)?),
        };
        self.matcher.matching(index, &request.query, &request.among)
    }
}

/// Looks up which sessions a query's words match, narrowing from the last
/// lookup when the query narrows it over the same sessions.
#[derive(Default)]
pub(crate) struct Matcher {
    /// The last lookup that succeeded.
    last: Option<LastSearch>,
}

/// A query looked up, the sessions it was looked up among, and those its
/// words matched.
struct LastSearch {
    query: Query,
    among: Arc<HashSet<SessionKey>>,
    matched: HashSet<SessionKey>,
}

impl Matcher {
    /// Which of `among` contain every word of `query` and none of its
    /// excluded words, as `index` holds them. A query that extends the last
    /// one over the same `among`, as when the person types on, costs no
    /// more than narrowing what that one matched. Fails only when the index
    /// can no longer be read.
    pub(crate) fn matching(
        &mut self,
        index: &Index,
        query: &Query,
        among: &Arc<HashSet<SessionKey>>,
    ) -> Result<HashSet<SessionKey>, DataError> {
        let candidates = match self.last.take() {
            // The same allocation is the same sessions: a new snapshot
            // brings a new set, and this one is kept alive by `last`.
            Some(last) if Arc::ptr_eq(&last.among, among) && query.narrows(&last.query) => {
                last.matched
            }
            _ => (**among).clone(),
        };
        let matched = index.matching(query, candidates)?;
        self.last = Some(LastSearch {
            query: query.clone(),
            among: Arc::clone(among),
            matched: matched.clone(),
        });
        Ok(matched)
    }
}

/// What a searcher and its stepper share.
#[derive(Clone)]
struct Shared {
    slot: Arc<Mutex<Slot>>,
    /// Wakes the thread waiting for the next request.
    wake: Arc<Condvar>,
    /// Set when the searcher is dropped; the thread ends before its next search.
    stop: Arc<AtomicBool>,
}

/// The ask waiting to be searched and the answer waiting to be taken.
#[derive(Default)]
struct Slot {
    request: Option<Request>,
    answer: Option<Answer>,
    /// The sequence number of the newest answer put in the slot, taken or not.
    answered: u64,
}

struct Request {
    seq: u64,
    query: Query,
    among: Arc<HashSet<SessionKey>>,
}

struct Answer {
    seq: u64,
    matched: Matched,
}

impl Slot {
    /// Puts `answer` in place of the one waiting, unless a newer one came
    /// first, as a query with no words answered while a search ran.
    fn answer(&mut self, answer: Answer) {
        if answer.seq > self.answered {
            self.answered = answer.seq;
            self.answer = Some(answer);
        }
    }
}

impl Shared {
    fn new() -> Shared {
        Shared {
            slot: Arc::new(Mutex::new(Slot::default())),
            wake: Arc::new(Condvar::new()),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        // A panic while holding the lock leaves a slot that is still whole.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits until a search is asked for, giving true, or the searcher is
    /// dropped, giving false.
    fn wait_for_request(&self) -> bool {
        let stopped = || self.stop.load(Ordering::Relaxed);
        let waited = self
            .wake
            .wait_while(self.slot(), |slot| slot.request.is_none() && !stopped());
        drop(waited.unwrap_or_else(PoisonError::into_inner));
        !stopped()
    }
}

impl Drop for Searcher {
    /// Asks the thread to end once the search it runs, if any, finishes,
    /// and returns without waiting for it.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Under the lock, so a thread about to wait can't miss it.
        let _slot = self.shared.slot();
        self.shared.wake.notify_all();
    }
}
