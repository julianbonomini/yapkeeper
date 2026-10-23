# Interface decisions

Yapkeeper opens as a full-width command palette: one query line, a result list, and a preview of the selected session. These decisions come from the disposable Ratatui prototype on the `prototype/tui-flow` branch (`prototypes/tui-flow/`), which was tried on synthetic sessions and then on the user's real Claude Code and Codex histories. Open questions are listed at the end.

## Layout

The palette fills the terminal inside a single rounded border. From top to bottom:

1. The query line, with the match count on the right (`2 / 783`).
2. A rule.
3. Results on the left (48%) and the preview on the right (52%) when the palette is at least 80 columns wide. Below that width the preview is dropped and `→` opens the conversation.
4. One footer line of key hints for the current context, replaced for three seconds by a confirmation message after an action.

There is no permanent location sidebar. The query line handles locations (see below).

Rejected during prototyping:

- A flat recent list with scope chips (All / Project / Worktree / Folder) above it. Chips do not scale to 20 or 30 projects, and the scope was easy to miss.
- A location tree beside the session list, with the preview in a third column. It navigated well but felt crowded, and the search scope depended on the selected tree row in a way that was hard to see.
- Two- and three-line cards. Harder to scan than one line per session.

## Query language

Words search conversation text, titles, and provider recaps. Every other token is a filter, and all filters must match.

| Form | Meaning | Examples |
|---|---|---|
| `@where` | Location: project, `project:worktree`, or any part of a path | `@acme-api`, `@acme-api:billing-retry`, `@scratch` |
| `#agent` | Provider | `#claude`, `#codex` |
| `is:state` | Session state | `is:orphaned`, `is:temp`, `is:git`, `is:folder`, `is:pr`, `is:recap`, `is:unavailable`, `is:hidden` |
| `!x`, `not:x` | Exclude a filter, an agent or state by name, or a word | `!codex`, `!temp`, `!@scratch`, `!is:pr`, `not:orphaned`, `!flaky` |

Rules:

- `@`, `#`, `is:`, `!`, and `not:` each open a completion list with session counts. Enter or Tab inserts the selected item.
- A space never opens a completion list. `is pr` is two search words, so any phrase stays searchable.
- No space goes between `!` and what it negates: `!codex`, not `! codex`.
- The query line colors filter tokens with the accent color and leaves search words plain, so the user can see how the query was read.
- Matching is case-insensitive substring matching with smart case. Fuzzy matching was not tried.
- Tool calls are searched only after `Ctrl+T`.
- Hidden sessions appear only with `is:hidden`.

## Locations

A location is a Git repo, one of its linked worktrees, or a plain folder. Yapkeeper derives it from the session's recorded working directory: it walks up to the nearest `.git`, treats a `.git` directory as a main checkout and a `.git` file as a linked worktree, and names the project after the main checkout's folder.

`Tab` (or typing `@`) opens the location list. Each project row comes first and covers the main checkout and every worktree; its worktrees are listed under it. Plain folders follow. Groups are ordered by most recent activity. Each row has a kind icon on the left, a kind label and path, and a session count. With the Unicode icon set:

```
▣ acme-api                git repo · 3 worktrees · ~/code/acme-api   10
   ⎇ billing-retry       worktree · ~/code/acme-api.wt/billing-retry  4
▢ ~/scratch/csv-cleanup   folder                                       1
⊘ ~/scratch/regex-tester  missing · folder no longer exists            1
```

Typing after `@` matches location names and paths, so `@code` finds every project under `~/code`. Result rows label locations as `project › worktree`; the `project:worktree` form is used only inside `@` tokens. `Ctrl+L` inserts the location Yapkeeper was launched from.

A folder that no longer exists cannot be checked for Git, so it is shown as `missing` with its path.

## Preview

The preview shows, in order:

1. Title: the provider title (Claude Code `custom-title`, then `ai-title`; Codex `thread_name`), or the first prompt.
2. Provider, age, message count, subagent count, and linked PR numbers.
3. Location icon and full directory, and the worktree name.
4. `orphaned` or `unavailable` when the session cannot resume in its recorded directory.
5. Recap: the latest Claude Code `away_summary`, when present.
6. Matches (while searching): up to three matching messages with the terms highlighted.
7. Opening: the first prompt.
8. Latest: the last prompt and the replies after it.

`→` opens the full conversation. `n` and `N` jump between matches, and `Enter` there opens the resume dialog.

Recaps cost nothing extra: Claude Code writes `away_summary` records into its session files. On the real histories, 180 of 783 sessions had one and 100 had a `pr-link` record. Codex sessions have no recap. Writing a summary for every session would need a model call during indexing; that is deferred (see open questions).

## Defaults and settings

`Ctrl+S` opens settings. Defaults:

| Setting | Default | Reason |
|---|---|---|
| Hide sessions in temp folders (`/private/tmp`, `/tmp`, `/var/folders`) | On | 59 of 783 real sessions were scratch work in temp folders. The header shows how many are hidden, and `is:temp` shows them anyway. |
| Hide missing folders in the location list | On | 46 of 82 real recorded folders no longer exist. They still appear when the user types a name after `@`. |
| List orphaned sessions after the rest | On | 501 of 783 real sessions were orphaned and crowded out live ones. They stay searchable. |
| Accent | Sage (256-color index 108), soft grey selection bar (index 237) | Chosen from cyan, amber, violet, sky, sage, and monochrome. |
| Location icons | Nerd Font | Chosen over plain Unicode (`▣ ⎇ ▢ ⊘`), text tags, and none. |

Terminals cannot report whether a Nerd Font is installed, so the icon set has to stay a setting with a Unicode fallback. `NO_COLOR` switches to the monochrome theme. Selection must remain visible without color (reversed or bold).

## Keys

| Key | Action |
|---|---|
| Typing | Edit the query |
| `↑` `↓`, `Ctrl+P` `Ctrl+N` | Move the selection |
| `Enter` | Resume dialog |
| `→` | Read the whole conversation |
| `Tab` | Location list |
| `Ctrl+L` | Insert the launch location |
| `Ctrl+X` | Hide or unhide the selected session |
| `Ctrl+T` | Include tool calls in search |
| `Ctrl+S` | Settings |
| `Ctrl+U` | Clear the query |
| `Esc` | Clear the query, then quit |
| `Ctrl+C` | Quit |

Every printable key goes to the query, so all actions use `Enter`, arrows, `Tab`, or `Ctrl` keys. This is why hide has no undo key in the palette: the toast points to `is:hidden` instead.

## Resume dialog

`Enter` on a result opens a dialog with the directory and the exact command, for example `claude --resume <id>` or `codex resume <id> -C <dir>`. A second `Enter` exits the TUI and starts the provider in that directory; the prototype only prints the command.

For an orphaned session the dialog explains that the folder is gone and offers `c` to resume in the launch directory instead. Resume never moves to another directory without that explicit choice. For Claude Code the dialog warns that the session may not be found from another directory, because Claude Code looks sessions up by project directory. For an unavailable folder (one that exists but cannot be inspected) it also offers `r` to check again.

## Loading

The prototype reads every history file at launch, in parallel threads, without an index. On the real histories (866 files, 1.18 GB) it loaded 783 sessions in about 0.3 s with 55 MB peak memory on the user's Mac. It skips tool results, keeps tool calls as one-line summaries, caps messages at 4,000 characters, and drops injected context such as command caveats, `AGENTS.md` instructions, and environment blocks. Sessions with no user prompt (64) and Codex subagent sessions (19) are left out.

## Open questions

- Project for orphaned worktrees. A deleted worktree's project could be inferred from its path (for example `~/.vago/worktrees/<project>/...`) or from the recorded Git branch. Today it falls back to the raw path.
- Cursor movement inside the query, so a filter in the middle can be edited.
- Fuzzy matching for search words and locations.
- Summaries for sessions without a recap: an opt-in, cached model call during indexing, which costs tokens and sends conversation text to a model.
- Where hides and settings persist. The prototype keeps them in memory; `docs/product.md` requires hides to survive index rebuilds.
- Behavior at 60 columns and 80×24 was built (the preview drops below 80 columns, and a "terminal too small" message appears below 48×12) but not reviewed by the user on real data.
- Plain CLI output mode, `Ctrl+Z` suspend, and mouse support were not prototyped.
