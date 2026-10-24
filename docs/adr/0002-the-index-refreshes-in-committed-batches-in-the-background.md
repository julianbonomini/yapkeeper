# The index refreshes in committed batches in the background

## Context

The palette has to open on cached results, index while it is open, stay responsive while it does, and share the index with a second Yapkeeper or a `yapkeeper list`. A refresh used to be one long `IMMEDIATE` transaction. That blocks all three: nothing it reads shows until the whole pass commits, a first launch shows nothing until every file is read, and another writer waits for the whole pass and fails after the 5 s busy timeout on a large first index.

## Decision

A refresh is a sequence of `IMMEDIATE` batches of at most 32 files or 16 MiB of new bytes, newest mtime first. A pass plans which files to read without a lock. Each batch then decides each file again inside its write lock, from a fresh stat and the row as stored then, so a file another writer read in the meantime is skipped or read on from where that writer stopped. Each batch still assembles every provider it touched from all of that provider's digests, so ADR 0001 stands.

`indexer.rs` runs the pass on a thread of its own that is never joined. After each commit it reads every session, with its location, on that thread and publishes it as one latest-wins snapshot; the palette takes it in when it next draws. A busy index ends the pass quietly; the next pass reads what it left.

Quitting and resuming don't wait for the pass. Each batch is one WAL transaction, so a batch the process ends in the middle of is discarded as a crash would be. The bundled SQLite opens its files with `O_CLOEXEC`, so the exec that resumes a session closes them and releases their locks.

## Rejected

A thread and channel owned by the terminal loop, sending indexing events to the palette. The pass's lifecycle would be split between two modules, and the riskiest code would sit where no test reaches.

The index as the bus: the palette polls `PRAGMA data_version` and reloads sessions and locations itself. That puts disk I/O on the UI thread after every commit.

Joining the worker on quit. It delays exit and the exec by up to a file read, for nothing SQLite's atomic commit doesn't already give.

## Consequences

- A snapshot taken mid-pass can lag on facts recorded in another file, such as Codex renames or subagent counts, until that file's batch commits. The finished pass matches a single full refresh.
- A `yapkeeper list` that is interrupted keeps the batches it committed.
- The palette redraws on a 50 ms tick while a pass runs.
- A later `Ctrl+R` or refresh interval wakes the same worker rather than adding a new seam.
