# Yapkeeper v1

Status: Rust, Ratatui, and SQLite selected; interface settled by prototype (see [interface.md](interface.md)); implementation workflow selection remains open.

## Purpose

Find a past coding conversation without remembering its provider, project, folder, or Git worktree. Understand the conversation and resume the selected session with its original provider in the correct directory within a few interactions.

Repository and command name: `yapkeeper`. Tagline: "Now, where was I?"

## Agreed behavior

- Support macOS and Linux first, using Rust with Ratatui and SQLite.
- Write first-party Rust using safe APIs only; enforce `forbid(unsafe_code)` in crate roots and workspace checks.
- Discover existing Claude Code, Codex, and Pi histories, including sessions started outside Yapkeeper. Adding a provider means adding an adapter; OpenCode is planned later.
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

## Interface

The prototype settled the interface; [interface.md](interface.md) records the decisions and open questions. Answers to the questions the prototype was built for:

- Global list or grouped browsing: a global command palette. Locations are filters typed into the query (`@project`, `@project:worktree`, `@path`), with a location list on `Tab`. Grouped project/worktree/folder browsing in a sidebar was tried and rejected.
- Distinguishing similar sessions: title, location (`project › worktree` or path) with a kind icon, age, and an `orphaned` or `unavailable` label on the row; provider, message count, PRs, and full directory in the preview.
- Recognizing a session: the provider recap when present, then matching passages while searching, then the opening prompt and latest exchange.
- Hidden, orphaned, and unavailable sessions: found with `is:hidden`, `is:orphaned`, and `is:unavailable`. Orphans sort after live sessions by default and say so in the row and preview.
- Narrow terminals: the preview drops below 80 columns and `→` opens the conversation; below 48×12 the palette shows a "terminal too small" message. Not yet reviewed on real data.

The list and preview should occupy stable space. Use flat separators and avoid nested borders or repeated status markers. Keyboard navigation, terminal restoration, width-aware text handling, and a plain CLI mode remain implementation requirements.

## Implementation and reuse

The selected stack is Rust, Ratatui, and SQLite. The README records the measurement-based rationale; the disposable benchmark code and reports have been removed. Actual TUI startup and behavior still need validation in the prototype.

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

Goals and their Tickets are written directly with `vago goal new` and `vago ticket add`.

The UI prototype is done (see [interface.md](interface.md)). Next work is slicing the implementation goal into tickets. Avoid maintaining a second Wayfinder decision map unless the planning grows beyond what this workspace can track.

Both workflows use Claude Code with Claude Opus 5.5. Reviews run as fresh agent steps. Terminal UI work must use `tui-design`; use `emil-design-eng` only when its interaction-polish principles help within terminal constraints.

Both workflows use `sh .vago/scripts/check.sh` for Cargo formatting, lint, safety, and test checks, including a final gate before merge. GitHub push, PR, and notification steps remain starter defaults to review before execution.

## Out of scope

Cloud sync, accounts, cross-provider conversation conversion, agent orchestration, worktree creation/deletion, and deletion of original provider histories.
