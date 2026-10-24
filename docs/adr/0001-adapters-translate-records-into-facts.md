# Adapters translate records into facts

## Context

`yapkeeper list` reads sessions from an index and opens only the history files that changed since the last run. A file that only grew must be read on from its last complete line, so the index needs a checkpoint for every file that it can resume from.

Some sessions are recorded across several files. Codex keeps renames in `session_index.jsonl`, apart from the rollout they name. Both Claude Code and Codex write each subagent's transcript to a file of its own, which belongs to a session recorded somewhere else. Reading one changed file must not mean rereading every file it joins.

## Decision

Adapters keep no state. `Adapter::read(path, from)` turns each complete record from byte `from` onward into provider-neutral facts (an identity, a working directory, activity, a message, a title, a recap, a linked PR, recorded Git) and returns the offset it reached. A fact depends only on its own record and the file's path, so the byte offset is the only checkpoint.

The model (`yapkeeper-model`) owns every fold rule: which title wins, that a file's first identity counts, which time is a session's activity, and how a subagent chain reaches its session. `SourceDigest` carries one file's fold between runs, and `assemble` turns a provider's digests into sessions. `FOLD_VERSION` names the rules.

`assemble` also says which session each thread belongs to (`Assembled::session_of`). The model never holds message text. The index stores each file's messages under the thread that file records and is the only code that attaches them to sessions. It also decides which messages are tool-level: tool calls, tool output and every thread that is not the session's own.

The index stores each file's identity (device and inode), size, mtime, offset, parser version, fold version and digest. It treats the digest as opaque text. A file whose identity, parser version or fold version changed, which shrank below its offset, or whose digest won't decode, is read again from the start.

## Rejected

Each adapter carrying its own serialised parse state, which the index stores and hands back on the next read. Every adapter would repeat the same resumption code with its own state type. That state's layout would leak into the index, which stores it. The adapter's result would also mix a cumulative part with only the messages that were new since the last read, which is hard to describe.

A second, in-memory path in the model (`scan`) that read every file from 0 and attached messages to sessions itself. Production didn't run it, so tests that drove it checked a copy of the rules rather than the rules. Adapter tests drive `Adapter::read`, and `list` and `search` over fixture roots, instead.

## Consequences

- A new provider may need a new `Fact` variant, which is a change to the model.
- Any change to a fold rule bumps `FOLD_VERSION`, and the next run then rereads every file once.
- Any change to what `read` gives from the same bytes bumps that adapter's parser version, and the next run rereads that provider's files.
- Any change in a provider's files assembles that provider's sessions again from all of its digests, at one small row per file.
- Adapters may also answer questions from provider metadata at call time (`is_running`) or about the provider's fixed behaviour (`resumes_by_directory`). They still keep no state.
