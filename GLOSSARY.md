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
A session whose recorded working directory no longer exists. A directory that cannot be inspected has an unknown availability state, rather than being presumed absent.

**Hidden session**:
A session the user has excluded from ordinary Yapkeeper results. Hiding is reversible and preserves the provider's original history.

**Subagent transcript**:
A conversation recorded for an agent acting within another session. A fork or a message's parent link alone does not establish this relationship.

**Preview**:
The conversation text displayed for a selected session before resuming it.
