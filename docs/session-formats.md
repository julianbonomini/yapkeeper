# Local session-format inspection

Observed read-only on 2026-10-23. These are structural findings from local samples, not stable provider APIs. No prompts, titles, private project paths, or transcript bodies are included here. Recheck current files and installed CLI help before implementing adapters.

## Claude Code

- Top-level path: `~/.claude/projects/<encoded-project>/<session-id>.jsonl`.
- Subagent path: `~/.claude/projects/<encoded-project>/<session-id>/subagents/agent-<id>.jsonl`, beside an `agent-<id>.meta.json`. Every record in a subagent transcript has `isSidechain: true`. Yapkeeper attaches each one to `<session-id>` and leaves out those with no `<session-id>.jsonl`.
- Message metadata includes `sessionId`, `cwd`, `gitBranch`, `timestamp`, `uuid`, `parentUuid`, `isSidechain`, and sometimes `agentId`.
- Yapkeeper takes the first non-empty `gitBranch` on a kept user or assistant record as the recorded branch. Claude Code records no remote.
- `parentUuid` also links ordinary messages; it does not prove that a transcript belongs to a subagent.
- `message.content` can be a string or an array of blocks. Observed blocks include `text`, `thinking`, `tool_use`, and `tool_result`.
- Title records include `custom-title` with `customTitle`, and `ai-title` with `aiTitle`. A `system` record with `subtype: away_summary` holds a recap in `content`; Yapkeeper keeps the latest. `pr-link` records carry `prNumber`, `prUrl`, and `prRepository`. Administrative and API records coexist with conversation records.
- Optional running-session metadata exists under `~/.claude/sessions/<pid>.json`; it is not the historical source. Each file carries `pid`, `sessionId`, `cwd` and `status`. Yapkeeper reads only the top-level `sessionId`, at most 64 KiB per regular `.json` file, to warn that a session is running. A listed pid isn't checked for liveness, so a stale file only adds a warning.
- No `sessions-index.json` was found locally. Discovery must work without one.

Installed CLI help supports `claude --resume <session-id>`. Launch it with the selected working directory as the process `cwd`. Claude Code looks sessions up by project directory, so resuming one from another directory may not find it. `--fork-session` creates a separate session and is not ordinary resume.

## Codex

- Active path: `~/.codex/sessions/<year>/<month>/<day>/rollout-<timestamp>-<id>.jsonl`.
- Archived path: `~/.codex/archived_sessions/<rollout-file>.jsonl`.
- Records have `timestamp`, `type`, `payload`, and frequently `ordinal`.
- Observed types include `session_meta`, `response_item`, `event_msg`, `turn_context`, `world_state`, `token_usage_record`, and `realtime_item`.
- `session_meta.payload` includes `id`, `session_id`, `cwd`, source information, timestamps, and `git` metadata. Multiple `session_meta` records can occur in one file.
- `session_meta.payload.git` has `branch`, `commit_hash`, and `repository_url`, and can be missing. Yapkeeper reads `branch` and `repository_url` from the first `session_meta` only, because a fork's copied parent meta follows its own.
- Subagent metadata includes `parent_thread_id` and `source.subagent.thread_spawn`, with parent identity, role, path, and depth. `source.subagent.other` also occurs. A parent can itself be a subagent; Yapkeeper follows the chain to the top session and leaves out a subagent whose chain reaches no session.
- `forked_from_id` and `history_base` occur. Shared or forked history must not automatically be classified as a subagent.
- Message response items have roles and block arrays, including `input_text` and `output_text` blocks.
- Events can echo conversational content; avoid indexing those echoes as duplicate messages.
- `session_index.jsonl` supplies `id`, `thread_name`, and `updated_at`. `history.jsonl` alone is insufficient to reconstruct conversations.
- A provider SQLite state database exists. Its private schema has not been selected as a dependency.

Installed CLI help supports `codex resume <session-id> -C <directory>`. Use the exact ID instead of the native picker's most recent session. Verify whether archived or otherwise unusual sessions can resume, rather than inferring it from successful parsing.

## Pi

Observed read-only on 2026-10-23 and 2026-10-24, aggregate counts only. Rechecked against the installed CLI's session manager on 2026-10-24.

- Session root: `PI_CODING_AGENT_SESSION_DIR` when set, otherwise `$PI_CODING_AGENT_DIR/sessions`, otherwise `~/.pi/agent/sessions`. `--session-dir` overrides all three for Pi itself.
- Under the default root, a top-level session is `<root>/<encoded-cwd>/<timestamp>_<session-id>.jsonl`, where `<encoded-cwd>` is the path with separators turned into `-` between `--` marks. A custom session directory holds session files directly, with no `<encoded-cwd>` folder. Yapkeeper reads `.jsonl` files in the root and one folder below it, and takes the working directory from the header rather than the folder name.
- The first record has type `session` with `version`, `id`, `timestamp`, and `cwd`. Some session files also carry `parentSession`. None of the 24 top-level files observed did.
- Other record types: `message`, `custom_message`, `custom`, `session_info` (its `name` is the session's title; Yapkeeper keeps the latest), `model_change`, `thinking_level_change`, and `compaction`. Some records in nested files have no `type`. Yapkeeper skips records without a `type` and records of types it doesn't know.
- Records link through `id` and `parentId`, forming a tree.
- `message.message` has a `role` (`user`, `assistant`, `toolResult`, `bashExecution`). `content` is a block array for `user`, `assistant` and `toolResult`; Pi's types also allow a plain string. Observed blocks are `text`, `toolCall` (`name`, `arguments`), and `thinking`. A `bashExecution` message has `command` and `output` instead of `content`. Yapkeeper keeps `user` and `assistant` text as ordinary text, `toolCall` blocks, `toolResult` text and `bashExecution` commands and output as tool text, and never indexes thinking.
- `custom_message` has `customType`, `content` (a string in every observed record), `display`, and sometimes `details`. Extensions write it, and Pi hands its content to the model as context. Observed kinds are subagent and background-task notices, mostly with `display: false`. Neither the person nor the assistant wrote it, so Yapkeeper keeps it as tool text.
- A session file `<timestamp>_<id>.jsonl` may have a folder of the same name beside it. Locally there were 136 run files, 107 fork files and 6 worker transcripts:
  - `<session-dir>/<run-id>/run-<n>/session.jsonl` is a subagent run. Its `session` header has its own `id` and no `parentSession`. Every observed run held one user prompt that the parent session's `subagent` tool call had written, and shared no entry ids with the parent.
  - `<session-dir>/forks/<timestamp>_<id>.jsonl` is a subagent started with a copy of the parent's context. Its header's `parentSession` is the path of the session file beside the folder. Every observed fork began with entries copied from the parent, with the same entry ids. Those that went on were handed a task from the parent's `subagent` tool call, and the parent's `subagent` tool results and `subagent-notify` messages name the fork's id.
  - `subagent-artifacts/` beside the session files holds a worker's input, output, metadata and `_worker_transcript.jsonl`. The transcript's records have `recordType`, `runId` and no `type`, and the file has no `session` header. Its run is named only inside other sessions' text, not by its path.
- Yapkeeper reads runs and forks as subagents of the session whose folder holds them, taking that session's id from the folder name, so they count toward its subagents and their text is tool-level. It finds them only through their session file, so a folder whose session file is gone is not read. It gives worker transcripts no thread, so they are left out. Where a run or fork sits classifies it, not its `parentSession`.

Installed CLI help supports `pi --session <path|id>` to open a specific session. Pi looks for the id among the starting directory's sessions first. When it finds the session under another project, it asks whether to fork it into the current directory. Launch it with the recorded `cwd` as the process working directory. `--fork` creates a new session and is not ordinary resume. Pi keeps no record of which sessions are open.

## OpenCode

Not yet supported. No OpenCode history was available locally to inspect.

## Adapter considerations

- Support configurable roots, including `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and Pi's session directory.
- Prefer recorded working directories to decoding folder names. Preserve the recorded branch separately from a worktree's current Git branch.
- Parse complete JSONL lines and retry an unfinished final line after more data arrives. Skip malformed complete records with bounded diagnostics.
- Handle unknown records, large lines, file replacement, truncation, repeated metadata, and growing histories.
- The index keeps each file's identity, size, mtime, offset, parser version and fold version. Incomplete bytes are never read: the next read starts at the last complete line. See [ADR 0001](adr/0001-adapters-translate-records-into-facts.md).
- Keep archived, orphaned, unreadable, forked, and subagent states explicit.
- Only an absent recorded directory establishes an orphan. Permission errors and unavailable filesystems require an unknown/unavailable state.

No malformed records appeared in the bounded samples, and inspected files ended with newlines when observed. Tests still need synthetic malformed records and incomplete tails.
