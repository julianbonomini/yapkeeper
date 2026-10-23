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
- Optional running-session metadata exists under `~/.claude/sessions/*.json`; it is not the historical source.
- No `sessions-index.json` was found locally. Discovery must work without one.

Installed CLI help supports `claude --resume <session-id>`. Launch it with the selected working directory as the process `cwd`. `--fork-session` creates a separate session and is not ordinary resume.

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

Observed read-only on 2026-10-23, aggregate counts only.

- Top-level path: `~/.pi/agent/sessions/<encoded-cwd>/<timestamp>_<session-id>.jsonl`. `--session-dir` overrides the root for Pi itself.
- The first record has type `session` with `version`, `id`, `timestamp`, and `cwd`. Some session files also carry `parentSession`.
- Other record types: `message`, `custom_message`, `custom`, `session_info` (its `name` is the session's title), `model_change`, `thinking_level_change`, and `compaction`. Some records in nested files have no `type`.
- Records link through `id` and `parentId`, forming a tree.
- `message.message` has a `role` (`user`, `assistant`, `toolResult`, `bashExecution`) and `content` as a block array. Observed blocks are `text`, `toolCall`, and `thinking`.
- Under a session's folder, `<session-dir>/<run-id>/run-<n>/session.jsonl` holds subagent runs, and `<session-dir>/forks/<timestamp>_<id>.jsonl` holds forks whose header names a `parentSession`. `subagent-artifacts/` beside the sessions holds worker transcripts and metadata. Verify which of these are subagent transcripts before classifying them; a `parentSession` alone does not establish it.

Installed CLI help supports `pi --session <path|id>` to open a specific session. Launch it with the recorded `cwd` as the process working directory. `--fork` creates a new session and is not ordinary resume.

## OpenCode

Not yet supported. No OpenCode history was available locally to inspect.

## Adapter considerations

- Support configurable roots, including `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and Pi's session directory.
- Prefer recorded working directories to decoding folder names. Preserve the recorded branch separately from a worktree's current Git branch.
- Parse complete JSONL lines and retry an unfinished final line after more data arrives. Skip malformed complete records with bounded diagnostics.
- Handle unknown records, large lines, file replacement, truncation, repeated metadata, and growing histories.
- Consider source file identity, byte offsets, size/mtime, and parser version in rebuildable index state. Decide and test how incomplete bytes are retained and retried.
- Keep archived, orphaned, unreadable, forked, and subagent states explicit.
- Only an absent recorded directory establishes an orphan. Permission errors and unavailable filesystems require an unknown/unavailable state.

No malformed records appeared in the bounded samples, and inspected files ended with newlines when observed. Tests still need synthetic malformed records and incomplete tails.
