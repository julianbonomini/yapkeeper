# Yapkeeper v1

Status: product direction agreed through the initial interview; UI, implementation stack, and workflow selection remain open.

## Purpose

Find a past coding conversation without remembering its provider, project, folder, or Git worktree. Understand the conversation and resume the selected session with its original provider in the correct directory within a few interactions.

Repository and command name: `yapkeeper`. Tagline: "Now, where was I?"

## Agreed behavior

- Support macOS and Linux first. Choose the language using measured startup, indexing, and search behavior. TypeScript is preferred where practical, and other languages are acceptable.
- Discover existing Claude Code and Codex histories, including sessions started outside Yapkeeper.
- Search all sessions globally. Support narrowing by project, Git worktree, and folder; global search must remain available regardless of the current location.
- Search ordinary user and assistant messages by default. Make tool calls and tool output an explicit search option.
- Let the user preview a conversation before resuming it.
- Exit the TUI and launch the selected provider in the same terminal, using the exact session identity and correct working directory.
- Preserve sessions when their recorded directory disappears and visibly mark them as orphaned.
- Let the user hide and unhide sessions, including orphans. Preserve the original transcripts and retain hide preferences when rebuilding the index.
- Distinguish top-level sessions from subagent transcripts.
- Keep session contents local. Read provider histories without modifying them and use a rebuildable local index/cache.
- Keep provider-specific logic separate from the shared model and interface.
- Tolerate partially written files, malformed records, unknown record types, and changes to source files.
- Use synthetic or redacted fixtures; never commit private transcripts.

## Speed and refresh

After indexing, opening Yapkeeper should show cached results immediately and searches should return results without perceptible delay. Opening the interface and searching must remain responsive while indexing runs.

Proposed measurement targets are a usable cached launch within 200 ms and search results within 50 ms. These are provisional targets, not measured results. Define the machine, corpus, timing boundaries, and percentiles before comparing implementations.

Proposed refresh behavior: show cached results first, scan changed histories in the background on launch, refresh periodically while open, and provide a manual refresh action. The interval, process lifetime, cancellation behavior, and handling of a source change during indexing still need validation. A full archive parse must not be required on each launch.

## UI prototype questions

The prototype should answer these questions with synthetic sessions:

- Does a global recent-session list with location filters work better than grouped project/worktree/folder browsing? Both must allow global search.
- What session and location information is needed to distinguish similar conversations quickly?
- Which conversation passages help the user recognize a session? A proposed starting point is the opening prompt and latest exchange while browsing, and a matching passage while searching.
- How should the user find hidden sessions and distinguish orphaned, unavailable, and resumable sessions?
- Can the dominant flow remain useful at 80 by 24 cells and in a 60-column tmux split? A narrow terminal needs a single-pane list/detail flow.

The list and preview should occupy stable space. Use flat separators and avoid nested borders or repeated status markers. Keyboard navigation, terminal restoration, width-aware text handling, and a plain CLI mode are implementation requirements to validate.

## Benchmark and reuse questions

Compare Rust/Ratatui, Go/Bubble Tea, and TypeScript with Bun and a suitable TUI framework before committing to a stack. Evaluate SQLite full-text search in each candidate. Measure initial indexing separately from cached launch, incremental refresh, search, and preview loading. Include large files, Unicode, growing files, malformed tails, and orphaned sessions. Read [the language performance investigation](language-performance.md) for evidence and measurement limits.

The local structural inspection found approximately 1,400 transcript files totaling 1.9 GB, with individual files approaching 60 MB. Counts changed during inspection because sessions were being written. Use generated data for repeatable benchmarks; inspect live data read-only when validating discovery.

The user subsequently authorized disposable benchmarks on actual histories. [The real-history comparison](real-history-benchmarks.md) measures full indexing, unchanged refresh, simulated tail catch-up, indexed search, peak resident memory, and a fivefold replay. Source histories remained read-only; private snapshots and indexes were removed. Rust and Bun had similar indexing throughput, with substantially lower peak RAM for Rust. Actual TUI behavior and the implementation language remain open.

Two candidates merit a focused evaluation:

- [ah](https://github.com/nihen/ah): Rust CLI with machine-readable discovery/search/preview output and resume support. It searches files directly, so the cost of a Yapkeeper cache and subprocess integration needs measurement.
- [sessions](https://github.com/nicknisi/sessions): Bun/TypeScript with SQLite full-text indexing. Its broader memory and archive features, subagent handling, and selected-session Codex resume behavior need evaluation against this scope.

No backend or library reuse decision has been made. Verify current licenses and source behavior before adopting code.

## Resume questions

Resolve these before making resume executable:

- What recovery action should be offered for an orphaned session? Ordinary resume must not silently use another directory.
- How should a selected session that is already active be handled?
- What metadata is authoritative when a source contains several working directories or metadata records?
- How should unavailable providers and unreadable directories appear?

Construct provider arguments as an argument array with an explicit process working directory. Do not execute shell commands taken from transcript text or generated display strings.

## Vago planning

The local planning item is `yapkeeper-v1`, using the Codex planning harness. Shared starter workflows are installed; no ready implementation goal or tickets have been created.

Next work is a UI prototype and a performance/reuse investigation. Their results should settle the stack and interaction model before slicing the implementation goal into tickets. Avoid maintaining a second Wayfinder decision map unless the planning grows beyond what this workspace can track.

The starter workflows assume Bun checks and a Claude Code implementation harness, and contain GitHub push, PR, and notification steps. These are starter defaults to review, not accepted choices for Yapkeeper.

## Out of scope

Cloud sync, accounts, cross-provider conversation conversion, agent orchestration, worktree creation/deletion, and deletion of original provider histories.
