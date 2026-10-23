# Real-history benchmark results

Measured on 2026-10-23 using a private, read-only snapshot of the user's actual Claude Code and Codex histories. Rust and Bun were close in indexing throughput; Rust used substantially less peak resident memory. Go's straightforward standard-library JSON implementation was slower on this workload. All three produced identical indexed content.

## Actual archive

The measured snapshot contained 1,439 transcript files, 1,909,030,219 bytes (1.91 GB), and 302,468 complete JSON records. These include subagent transcripts. Files with incomplete final records were snapshotted only through their final newline; provider histories were never modified.

The shared extraction rules selected 20,983 user/assistant text messages totaling 45,130,594 UTF-8 bytes. Each final SQLite index occupied 73,035,776 bytes. Tool output, thinking, images, administrative records, and event echoes were excluded from the search index. The programs still read and decoded the complete 1.91 GB snapshot.

## Measured comparison

These are the final runs after fixing unbuffered Rust manifest reading. Three runs per language were performed in rotated order. All native executables were built with optimized settings and warmed using a tiny synthetic corpus before measurement.

| Workload | Rust | Go | TypeScript/Bun |
| --- | ---: | ---: | ---: |
| Initial index, median | 2.75 s | 6.44 s | 2.87 s |
| Initial index, median peak RSS | 50.3 MiB | 48.4 MiB | 235.3 MiB |
| Initial index, maximum peak RSS | 50.3 MiB | 51.3 MiB | 272.5 MiB |
| Unchanged refresh, median | 15.1 ms | 16.8 ms | 24.3 ms |
| Replay a 43.5 MB tail backlog, median | 111 ms | 221 ms | 123 ms |
| Ranked search, query p95 | 3.72 ms | 3.74 ms | 3.68 ms |

An unchanged refresh reads saved checkpoints and stats every manifested file, returning without reading transcript content. There were 21 measurements per language. Initial indexing, refresh, and catch-up times are measured from process launch through exit, including database setup and final close/checkpoint work. Source directory enumeration is excluded because each candidate receives the same manifest. The earlier metadata-only scan of the actual directory trees was around 29 ms when warmed; adding it to these measurements is not an end-to-end TUI benchmark.

Catch-up is a controlled simulation using real history: the runner backs up the full temporary index, removes the indexed tails of 53 recently modified files, rewinds their checkpoints to record boundaries approximately 1 MiB from each file's end, and replays those tails. It excludes the backup/rewind setup from timing. The replay restores the exact original indexed content. This is not a measurement of how many bytes were appended during a real 24-hour period.

Search uses 16 generic query shapes repeated eight times, including common terms, a phrase, conjunctions, and a prefix query. The first 16 queries per process are omitted from the query-latency summary, leaving 336 timed samples per language. Database opening and statement preparation are outside individual query timings. Query result counts and checksums matched. Search time excludes filtering, session deduplication, snippets, and UI rendering.

## Larger replay

A separate single-round growth probe replayed the real snapshot five times under distinct benchmark file IDs, processing 9.55 GB across 7,195 file entries and creating 104,920 indexed messages. This repeats existing content; it is not 7,195 distinct genuine conversations or a representative prediction of future history. The new snapshot differed slightly from the baseline because live sessions continued writing between runs.

| Fivefold replay, one initial-index run | Rust | Go | TypeScript/Bun |
| --- | ---: | ---: | ---: |
| Initial index | 14.53 s | 34.00 s | 14.91 s |
| Peak RSS | 68.8 MiB | 61.7 MiB | 283.4 MiB |
| Unchanged refresh, median of seven runs | 35.6 ms | 40.5 ms | 42.1 ms |
| Ranked search, query p95 | 25.8 ms | 25.6 ms | 25.0 ms |

The same indexed-content and catch-up checks passed. Processing five times the volume increased Bun's peak RSS modestly, rather than fivefold. Its larger footprint in the baseline is not evidence that all source histories accumulate in JavaScript memory. Rust retained its lower footprint. This is one scaling check, not a memory-leak test or a guarantee for much larger or differently shaped archives.

## What these results establish

The Rust probe used roughly one fifth of Bun's peak resident memory while achieving similar initial-index throughput. This is a measured advantage of these implementations. Bun's observed 208 to 273 MiB range is larger than Rust's, but it does not amount to loading the 1.91 GB archive into RAM. The streaming design keeps source volume separate from retained application data.

Initial indexing takes seconds in either fast candidate. Rebuilding the whole archive before every launch would not meet the intended instant-open experience. Saved checkpoints make unchanged opens and moderate catch-up much cheaper. These measurements support investigating on-demand incremental refresh; a daemon remains useful when index freshness while the TUI is closed is a requirement. Neither daemon service management nor a complete catch-up system was implemented here.

The results leave a practical language tradeoff. Rust provides a smaller measured working footprint. TypeScript/Bun remains viable for throughput and indexed search if its higher footprint is acceptable. The benchmark does not measure which language an LLM writes more reliably or how quickly the production tool can be maintained. Go's result measures its standard dynamic JSON decoder and this cgo binding, rather than the best achievable Go implementation.

## Boundaries and verification

The machine was the same Apple M4 Max with 128 GB RAM used in the earlier probes. Versions were rustc 1.98.1, Go 1.27.1, Bun 1.4.2, and SQLite 3.54.0. The filesystem cache was warm after snapshot creation; storage-cold behavior and Linux were not measured. SQLite used WAL, synchronous NORMAL, an 8 MiB configured page cache, prepared inserts, and transactions every 1,000 selected messages.

Each candidate streams one file at a time, decodes generic JSON, and uses the same extraction rules and SQL. Rust uses serde_json and direct SQLite C calls, Go uses encoding/json and cgo calls, and Bun uses JSON.parse and bun:sqlite. These are component implementations, not an exhaustive language ranking. Their allocation and binding strategies differ. No effort was made to exhaustively tune any candidate.

Validation compared aggregate record/message/byte counts and SHA-256 digests of ordered indexed rows, including their file/record offsets, roles, and full text. Initial indexes and all recovered tail indexes matched. No malformed records appeared in the fixed real snapshot. Synthetic smoke data separately covered malformed records, Unicode, embedded NUL characters, CRLF, ignored tool/event records, large records, unchanged refresh, and search.

Peak RSS comes from the OS child resource report returned by wait4, not a language-specific heap metric. It includes native SQLite and runtime allocations. This does not measure steady-state memory after a long-lived daemon finishes indexing, input responsiveness while indexing, renderer startup, or a complete TUI.

The runner created anonymous snapshot files and private indexes in a mode-restricted temporary directory. They were removed after the run. Only benchmark source and aggregate results remain in the repository. Original filenames, directories, conversation titles, prompts, and transcript text were not written to the results or exposed in tool output.

Source and reproduction instructions: [benchmark directory](../benchmarks/real-history/). Raw aggregate results: [baseline](../benchmarks/real-history/results.json) and [fivefold replay](../benchmarks/real-history/results-growth-5x.json).
