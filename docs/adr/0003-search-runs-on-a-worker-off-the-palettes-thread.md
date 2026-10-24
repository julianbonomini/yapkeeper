# Search runs on a worker off the palette's thread

## Context

The palette searched on its own thread, so a slow search held back the frame that shows the key just typed: typing waited on the index. The bench's typing timings ended at the first frame after the key, which was also the frame with the results only because the search ran before it. ADR 0002's indexer already shows how this codebase moves work off the palette's thread: one module owns the thread, its own connection and a latest-wins hand-over, and the palette takes what was handed over when it next draws.

## Decision

`Catalog::searching_in_background` gives the catalog a `searcher.rs` worker: a thread that is never joined, with its own read-only connection to the index, opened by `Index::reading`, which creates, recreates and writes nothing. The catalog still owns what a search means: its sessions, the hides, and turning matched keys into ordered results.

The hand-over is latest-wins both ways. An ask whose search hasn't started is replaced by the next. Only the newest answer for the query last asked is handed over, so results for a query the line no longer shows are thrown away, never drawn. A query with no words reads nothing from the index, so it is answered at once, without the worker, but still through the hand-over.

The palette asks on every key and takes answers only when it draws, so the keys that act on a result (`Enter`, `Ctrl+O`, `Ctrl+X`, the arrows) act on the rows on screen. While a search is in flight it redraws on a 10 ms tick, and says `searching ·` once a search has run for 150 ms.

Narrowing from the last lookup lives in `Matcher`, used by both the catalog's own `search` (the CLI, and catalogs with no worker) and the worker. It narrows only over the same set of sessions, which the catalog replaces whenever a snapshot is applied.

## Rejected

The terminal loop owning the thread or the channel, for ADR 0002's reasons: the worker's lifecycle would be split between two modules, where no test reaches it.

The palette holding a `Searcher` beside its `Indexer`: it would have to call four methods in the right order, and which answers count would leak into it.

A full search engine on the worker, with every session mirrored there: the same state would have two owners, to save under 1 ms of filtering.

Moving the catalog's connection to the worker: the preview and the conversation read the index on the palette's thread.

Joining the worker on quit, or interrupting a running search: a search only reads, so it may simply finish.

Dropping every answer asked before a snapshot: during a first index, snapshots can arrive faster than a search finishes, and results would never show.

## Consequences

- Results lag the query line by up to one search and a 10 ms tick.
- With the worker, keys act on the drawn results, including in a batch: a key in the same batch as typing acts on the frame drawn before the batch.
- A catalog with no worker, as in most tests, answers at once, and its keys act on the batch's query.
- The palette holds a second connection to the index while it is open.
- After a snapshot, the results for the query on the line are shown with the current hides, and the sessions the snapshot added are missing until the search asked after it arrives.
- Quitting and resuming leave the worker detached; `O_CLOEXEC` closes its files on exec, as in ADR 0002.
