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

**Location list**:
The list `Tab` or `@` opens: one row per project, with its worktrees under it, then one per plain folder, each with how many sessions its `@` token lists. Choosing a row puts that token in place of the `@` token being typed.

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
The provider-specific part of Yapkeeper that names the directory a provider's history is under, discovers the history files there, reads it without modifying it by turning each record into facts, builds the resume command, and says whether the provider lists a session as running and whether it finds sessions by directory. The shared model folds facts into sessions; the index stores those sessions with each file's messages, and the interface sees only sessions.

**Index**:
Yapkeeper's rebuildable copy of every session and its searchable text, in `index.sqlite`. Deleting it loses nothing; the next listing builds it again.

**Preferences**:
The user's own choices, hidden sessions and settings, in `prefs.sqlite`, kept apart from the index so rebuilding never loses them.

**Settings**:
The preferences set with `Ctrl+S` that change what the palette lists and how it looks: hiding temp sessions, hiding missing folders, listing orphans last, the accent and the location icons. The command line ignores them.

**Title**:
The name shown for a session: the one the user set, else the one the provider generated, else the first prompt the user typed. Injected context never counts as a prompt.

**Palette**:
The full-screen view `yapkeeper` opens on a terminal: a query line, the matching sessions, a preview of the selected one, and key hints, searched live as the user types.

**Preview**:
The conversation text displayed for a selected session before resuming it.

**Launch directory**:
The directory `yapkeeper` was started in. `Ctrl+L` puts its `@` token at the cursor, in place of the `@` token there, and `c` in the resume dialog resumes there.

**Resume dialog**:
What `Enter` opens on a session: the exact command and directory resuming would use, any warnings, and why it can't start when it can't.
