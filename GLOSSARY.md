# Yapkeeper

Yapkeeper finds coding conversations and the locations where they took place.

## Language

**Session**:
A conversation recorded by a coding agent, identified within that provider's history. A session remains a session when its original working directory disappears.

**Provider**:
The coding agent that records and resumes a session, initially Claude Code, Codex, or Pi.

**Working directory**:
The filesystem directory recorded for a session. It may be a Git worktree or an ordinary folder.

**Location**:
Where a session ran, derived from its working directory: a Git repo's main checkout, one of its linked worktrees, or a plain folder. A project groups a repo's main checkout and its worktrees.

**Orphaned session**:
A session whose recorded working directory no longer exists. A directory that cannot be inspected makes it an unavailable session instead.

**Unavailable session**:
A session whose working directory exists but cannot be inspected, such as when permission is denied. It is never presumed orphaned.

**Temp session**:
A session whose working directory is under a system temp folder.

**Hidden session**:
A session the user has excluded from ordinary Yapkeeper results. Hiding is reversible and preserves the provider's original history.

**Subagent transcript**:
A conversation recorded for an agent acting within another session. A fork or a message's parent link alone does not establish this relationship.

**Search word**:
A whitespace-separated piece of the query that is not a **Filter**. A session matches it when its title, recap or a conversation message contains it; a word with an uppercase letter must match case.

**Filter**:
A query token that isn't a search word: `@where`, `#provider` or `is:state`, optionally negated with `!` or `not:`. A session must match every filter.

**Tool-level text**:
Tool calls, tool output and subagent transcripts: text that belongs to a session but is not the person and the assistant talking. Searched only on request.

**Adapter**:
The provider-specific part of Yapkeeper that discovers a provider's history, reads it without modifying it by turning each record into facts, and builds the resume command. The shared model combines facts into sessions; the index and interface see only sessions.

**Index**:
Yapkeeper's rebuildable copy of every session and its searchable text, in `index.sqlite`. Deleting it loses nothing; the next listing builds it again.

**Preferences**:
The user's own choices, such as hidden sessions, in `prefs.sqlite`, kept apart from the index so rebuilding never loses them.

**Title**:
The name shown for a session: the one the user set, else the one the provider generated, else the first prompt the user typed. Injected context never counts as a prompt.

**Preview**:
The conversation text displayed for a selected session before resuming it.
