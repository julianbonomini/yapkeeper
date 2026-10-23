# Language and TUI performance investigation

Follow-up: [benchmarks using the actual 1.91 GB history archive](real-history-benchmarks.md) now measure streaming indexing, catch-up, search, and peak memory. Rust and Bun have similar indexing throughput in those probes, with substantially lower peak RSS for Rust. The synthetic component results below are the earlier investigation, not the final evidence for archive-scale behavior.

The earlier investigation below was performed on 2026-10-23. In those synthetic probes Rust started the cached-list command about 6 ms sooner than Bun, all three had similar indexed search latency, and Bun decoded the synthetic JSONL fastest. Those differences did not establish a user-visible winner against the proposed 200 ms launch and 50 ms search budgets. The follow-up above now supplies actual-scale indexing and memory measurements; a complete TUI remains unmeasured.

Rust/Ratatui is the alternative to prefer if allocation control and avoiding a tracing garbage collector outweigh the existing TypeScript preference. Go/Bubble Tea remains a viable middle option. This recommendation is an engineering judgment from the measurements and framework properties below, not a completed production-stack benchmark. GoLand is an IDE; the language under consideration is Go.

## Actual archive scale and refresh

A subsequent read-only filesystem inventory on the same day found 1,436 transcript files totaling approximately 1.90 GB (1.77 GiB): 1,165 Claude Code files at 1.48 GB, and 271 active/archived Codex files at 0.43 GB. The largest file was approximately 59.5 MB. The Claude count includes 339 subagent-path files; Codex child sessions require metadata classification, so transcript file count is not a count of top-level conversations. Files remained active during inspection, and totals changed slightly between observations.

In that snapshot, 55 files had modification times within the previous 24 hours. Their current whole-file sizes totaled approximately 174 MB. This is not the amount of newly appended text; that requires comparing against saved indexing checkpoints.

The initial Python directory enumeration and metadata inspection took approximately 216 ms. Seven subsequent recursive `scandir`/`stat` passes took 28 to 34 ms, with a median of 29.14 ms and no reported filesystem errors. These warmed passes counted files and sizes only. They excluded transcript reads, parsing, source validation, SQLite updates, Git inspection, and TUI rendering. They do not establish a startup-refresh budget.

The 42 MB synthetic parsing probe is much smaller than the existing archive and did not measure index construction or memory. It is insufficient evidence that any candidate will scale acceptably. Next measurements need streaming initial indexing at the existing archive scale, realistic incremental catch-up after short and long gaps, and a larger growth case, while measuring peak memory and interactive latency.

A persistent index avoids reparsing unchanged histories. Saved file identity and validated offsets can avoid reparsing old bytes in append-only files; replacements, truncations, and format/parser changes need recovery. A daemon could run the same incremental indexer on file-change events with periodic reconciliation. An on-demand process catches up when the TUI opens. The choice changes freshness and process management; either design still needs incremental indexing. It has not been selected yet.

## Local measurements

All measurements used generated data, without reading provider histories. Source and raw samples are in [`benchmarks/language-comparison`](../benchmarks/language-comparison/). The generator produced 80,000 ASCII JSONL records, 42,221,120 bytes (40.27 MiB), and a 47,833,088-byte SQLite database with 10,000 session summaries and 80,000 indexed messages.

| Probe, milliseconds | Rust median / p95 | Go median / p95 | TypeScript/Bun median / p95 |
| --- | ---: | ---: | ---: |
| Process launch, indexed first 20 sessions, output, and exit | 7.70 / 9.46 | 8.65 / 11.75 | 13.27 / 16.14 |
| Read and dynamically decode the 40.27 MiB JSONL file | 43.13 / 43.86 | 128.21 / 132.99 | 28.86 / 31.00 |
| Prepared ranked FTS query and copy up to 20 results | 3.96 / 4.28 | 4.04 / 4.32 | 3.86 / 4.13 |

Environment: Apple M4 Max, 16 CPU cores, 128 GB RAM, macOS 27.0.1 arm64; rustc 1.98.1, Go 1.27.1, Bun 1.4.2. SQLite reported 3.54.0 through all three bindings. Rust used a release Cargo build for JSON and `rustc -O` for SQLite; Go used compiled binaries; Bun executed TypeScript directly. Builds and fixture generation were outside all timing intervals. The machine was not isolated from other work.

The cached-list probe measured from the Python parent before direct subprocess creation until the child exited, capturing stdout through a pipe. It includes process launch, driver setup, one indexed query, copying and printing 20 rows, and shutdown. It excludes TUI imports, terminal initialization, layout, rendering, terminal emulator paint, and input readiness. Treat it as a useful lower bound, not time to a usable TUI. Three rotating warmup rounds preceded 51 recorded runs per candidate, with candidate order rotated each round.

JSON timing starts inside the child, immediately before reading the file, and includes decoding every record and summing extracted content lengths. Rust used `serde_json::Value`, Go used `encoding/json` into `map[string]any`, and Bun used `JSON.parse`. All produced 80,000 records and the same 29,410,000-byte extracted-content checksum. Three warmups preceded 15 timed runs per candidate. The reported p95 is the nearest-rank sample percentile; with 15 samples it is the maximum observed value, not a stable population-tail estimate.

This is whole-file buffering and generic object materialization. Go's `bytes.Split`, JavaScript string splitting, and Rust line iteration allocate differently. It is neither a streaming incremental adapter nor a complete indexing benchmark: there are no index inserts, source discovery, malformed records, large individual records, or provider-specific extraction rules. Typed decoding, borrowed data, alternative parsers, Unicode, and streaming could change the ordering. The result specifically contradicts the assumption that Rust always parses this kind of JSON faster than Bun.

FTS timing is inside an already-running process and includes binding, executing, and copying result values, but excludes opening the database and preparing the statement. All candidates used the same database and SQL: message-level `MATCH`, ordering by `rank`, and a limit of 20. Five processes each ran the same seeded sequence of 192 single-term queries; the first 16 timings in each process were discarded, leaving 880 samples per candidate. Returned row counts and checksums matched. The corpus has 16 similarly common terms, so this does not cover arbitrary selectivity, broad queries, session-level deduplication, filters, or snippet generation.

Rust and Go used small direct bindings to the system SQLite C API; Bun used `bun:sqlite`. This measures native SQLite plus those particular bindings, not idiomatic production Rust or Go database drivers. Although all reported the same SQLite version, build options and binding behavior may differ. The sub-millisecond differences are insufficient evidence of a language advantage for indexed search. FTS5 provides indexed matching and ranking in SQLite; the host language does not implement that search algorithm. See [SQLite's FTS5 documentation](https://www.sqlite.org/fts5.html).

The OS file cache was warm. Cold storage behavior, Linux, packaged executables, sustained background indexing alongside input, and real terminal rendering were not measured. Resource accounting was denied by the environment, so there are no measured peak-RSS or GC-pause comparisons. Do not extrapolate the 40 MiB parse directly to the existing 1.9 GB archive.

## Framework and language tradeoffs

| Candidate | Reasons to choose it | Costs to validate for Yapkeeper |
| --- | --- | --- |
| Rust + Ratatui | Compiled native code and ownership without a garbage collector; explicit control over buffers and event processing. The local cached-list probe was fastest. | Ownership and lifetime design add development decisions; the app owns event scheduling and background work. Measure the full renderer, database driver, and release packaging rather than assuming native code guarantees the target. |
| Go + Bubble Tea | Compiled binary, standard JSON/filesystem/process tools, and a model/update/view architecture with commands returning I/O results as messages. | Allocations interact with GC and scheduling. Standard dynamic JSON decoding was slower in this probe, though that work belongs off the UI path. SQLite driver choice affects both performance and packaging. |
| TypeScript + Bun + OpenTUI or Ink | Matches the stated language preference, provides native JSON decoding and built-in SQLite, and supports familiar declarative UI development. The local parse and FTS probes met the proposed budgets with room remaining. | Keep synchronous JSON/SQLite work off the input/render path. Runtime, renderer imports, layout, GC, and native package distribution all need end-to-end measurement. React/Yoga and OpenTUI are different stacks and must not be grouped into one measured result. |

Rust's ownership model provides memory safety without requiring a garbage collector; it does not by itself establish lower total memory use. Ratatui is immediate-mode: the application decides when to draw, recreating the frame from state. Those properties give control, but the application still needs bounded rendering and background I/O. See [the Rust ownership chapter](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html) and [Ratatui rendering](https://ratatui.rs/concepts/rendering/).

Bubble Tea separates the model, update messages, commands, and view. Its runtime handles redrawing the view. Go's garbage collector does most work concurrently and is designed to limit pauses; its documentation describes CPU, memory, and latency tradeoffs. GC is a cost to measure under sustained indexing, not evidence that a session picker will stutter. See [Bubble Tea's primary documentation](https://github.com/charmbracelet/bubbletea) and [Go's GC guide](https://go.dev/doc/gc-guide).

Ink uses React and Yoga for terminal layout. OpenTUI instead supplies a Zig native core with TypeScript bindings and optional React/Solid renderers. A TypeScript application can therefore spend much of its database, parsing, and painting time in native implementations. Neither project's architecture establishes how fast Yapkeeper's first frame will appear. See [Ink](https://github.com/vadimdemedes/ink) and [OpenTUI](https://github.com/anomalyco/opentui).

Bun's built-in SQLite interface is synchronous. Indexing and potentially expensive searches must run away from the UI event loop; Bun supports worker threads, while its current documentation still marks the Worker API experimental, particularly termination. A dedicated process is another design to evaluate. See [Bun SQLite](https://bun.sh/docs/runtime/sqlite) and [Bun workers](https://bun.sh/docs/runtime/workers).

Distribution also depends on the stack. Bun can produce a standalone executable that embeds its runtime, so TypeScript does not necessarily require users to install Node or Bun. That differs from removing runtime overhead. For Go, a C-backed SQLite driver introduces cgo/toolchain considerations; a pure-Go driver has different costs and needs its own measurement. See [Bun executable packaging](https://bun.sh/docs/bundler/executables) and [Go's cgo documentation](https://pkg.go.dev/cmd/cgo). Binary sizes, install sizes, and build times were not compared here.

## Original follow-up plan

The measured native-process advantage was about 6 ms over Bun, and indexed FTS was effectively comparable for this corpus. Those results support keeping TypeScript/Bun in contention; they do not justify paying for a language change solely to improve cached search. They also do not prove that an Ink or OpenTUI application will meet the launch target.

Before locking the stack, measure a small release-built session-list/preview prototype from process launch to its first complete, input-ready frame in a PTY. Include the actual database driver and TUI framework, at 80 by 24 and 60 by 24 cells. Measure input-to-updated-frame p50/p95 while a separate worker refreshes generated histories, plus idle CPU and peak RSS. Include a plain CLI path so framework initialization can be separated from storage work. Use the same visible rows and preview text across candidates.

The [real-history follow-up](real-history-benchmarks.md) has now compared streaming indexing and refresh, actual-scale data, a larger replay case, index writes, bounded batches, and measured memory. The remaining UI question is how the same visible screen behaves under background work in the candidate TUI stacks. No production framework or language has been selected.

## Reproduce

From the repository root:

```sh
python3 benchmarks/language-comparison/run.py
```

The runner needs Python with SQLite FTS5, Rust/Cargo, Go, Bun, and a C toolchain with SQLite headers/library for the native bindings. Cargo runs offline and requires `serde_json` 1.0.151 and its dependencies already cached. It adds no project dependencies and downloads nothing. It creates synthetic input, database, Cargo project, Go build cache, and binaries in a temporary directory, then removes them. The result file is overwritten with the new measurement. The optional RSS probe uses macOS `/usr/bin/time -l` and reports unavailable measurements when resource accounting is denied. No production provider history is accessed.
