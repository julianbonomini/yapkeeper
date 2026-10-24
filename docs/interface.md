# Interface decisions

Yapkeeper opens as a full-width command palette: one query line, a result list, and a preview of the selected session. These decisions come from the disposable Ratatui prototype on the `prototype/tui-flow` branch (`prototypes/tui-flow/`), which was tried on synthetic sessions and then on the user's real Claude Code and Codex histories. Open questions are listed at the end.

## Layout

The palette fills the terminal inside a single rounded border. From top to bottom:

1. The query line, with the match count on the right (`2 / 783`), after `searching ·` while a search runs long, or `indexing ·` while a background pass runs.
2. A rule.
3. Results on the left (48%) and the preview on the right (52%) when the palette is at least 80 columns wide. Below that width the preview is dropped; `Ctrl+O` opens the conversation at any width.
4. One footer line of key hints for the current context, replaced for three seconds by a confirmation message after an action. Under the results the hints include `^O read`, `Tab locations` and `^S settings`; while a completion list is open, `Tab insert`.

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
| `#provider` | Provider | `#claude`, `#codex`, `#pi` |
| `is:state` | Session state | `is:orphaned`, `is:temp`, `is:git`, `is:worktree`, `is:folder`, `is:pr`, `is:recap`, `is:unavailable`, `is:hidden` |
| `!x`, `not:x` | Exclude a filter, a provider or state by name, or a word | `!codex`, `!temp`, `!@scratch`, `!is:pr`, `not:orphaned`, `not:worktree`, `!flaky` |

Rules:

- `@`, `#`, `is:`, `!`, and `not:` each open a completion list with session counts, for the token the cursor is in or at the end of, anywhere in the query. Enter or Tab puts the selected item in place of that whole token, keeps the rest of the query, and leaves the cursor after the one space that follows it.
- A space never opens a completion list, and neither does the cursor on a space or at a token's start. `is pr` is two search words, so any phrase stays searchable.
- No space goes between `!` and what it negates: `!codex`, not `! codex`.
- The query line colors filter tokens with the accent color and leaves search words plain, so the user can see how the query was read.
- Matching is case-insensitive substring matching with smart case. Fuzzy matching was not tried. A word with an uppercase letter matches case; other words ignore case letter by letter, so the index and the check always agree.
- Tool calls, tool output and subagent transcripts are searched only after `Ctrl+T`, or with `--tools` on the command line.
- Hidden sessions appear only with `is:hidden`.

## Locations

A location is a Git repo, one of its linked worktrees, or a plain folder. Yapkeeper derives it from the session's recorded working directory: it walks up to the nearest `.git`, treats a `.git` directory as a main checkout and a `.git` file as a linked worktree, and names the project after the main checkout's folder.

A session whose directory no longer exists is `orphaned`; one whose directory exists but can't be inspected, such as when permission is denied, is `unavailable`. Both are read from the disk on every listing and never stored, so deleting a directory takes effect on the next listing. `yapkeeper list` prints each session's location as `project`, `project › worktree`, or the folder path, after an `orphaned` or `unavailable` label when one applies.

An orphaned or unavailable session's project is inferred on a best-effort basis. The first of these that applies names the project: the recorded `repository_url`, then the nearest existing repo above the directory, then a path layout (`X.wt/w`, `R/.claude/worktrees/w`, `~/.tool/worktrees/P/w`, `P/worktrees/w` or `P/.worktrees/w`). The worktree comes from that layout, or from a recorded branch whose last segment matches the folder name. When nothing applies, the raw path is shown.

`Tab` (or typing `@`) opens the location list. Each project row comes first and covers the main checkout and every worktree; its worktrees are listed under it. Plain folders follow. Project groups come first, then folders, each ordered by most recent activity, with missing places last. Each row has a kind icon on the left, a kind label and path, and a session count. A row's count is the number of sessions its `@` token lists, so a folder also counts its subfolders' sessions. A folder row shows its path once, as its label, with `folder` as its kind. With the Unicode icon set:

```
▣ acme-api                git repo · 3 worktrees · ~/code/acme-api   10
   ⎇ billing-retry       worktree · ~/code/acme-api.wt/billing-retry  4
▢ ~/scratch/csv-cleanup   folder                                       1
⊘ ~/scratch/regex-tester  missing · folder no longer exists            1
```

The kind words are never cut. In a narrow terminal the path gives way from the left, and a missing folder's `folder no longer exists` from the right.

Typing after `@` matches location names and paths, so `@code` finds every project under `~/code`. Result rows label locations as `project › worktree`; the `project:worktree` form is used only inside `@` tokens. `Ctrl+L` inserts the location Yapkeeper was launched from.

`Tab` and `Ctrl+L` put their `@` token at the cursor, after the token the cursor is in so no word is split, with one space between it and the tokens either side. When the cursor is on an `@` token, from its first character to just after its last, theirs takes its place, so changing location never leaves two location filters; a negated `!@where` is an exclusion and stays. `Tab` leaves the cursor after its `@`, so the list stays open; `Ctrl+L` leaves it after the space that follows the token.

In the location list, a folder that no longer exists cannot be checked for Git, so it is shown as `missing` with its path. A missing folder reads `missing · folder no longer exists`. Missing places are left out while nothing follows `@`, and shown once anything does.

## Preview

The preview shows, in order:

1. Title: the provider title (Claude Code `custom-title`, then `ai-title`; Codex `thread_name`; Pi `session_info` name), or the first prompt.
2. Provider, age, message count, subagent count, and linked PR numbers.
3. Location icon and full directory, and the worktree name.
4. `orphaned` or `unavailable` when the session cannot resume in its recorded directory.
5. Recap: the latest Claude Code `away_summary`, when present.
6. Matches (while searching): up to three matching messages with the terms highlighted.
7. Opening: the first prompt.
8. Latest: the last prompt and the replies after it.

`Ctrl+O` opens the full conversation in place of the query line, results and preview, with the session's title and where it ran above it. Every message is shown whole, with the search words highlighted, and each tool call is one line. `↑` `↓` scroll by a line, `PgUp` `PgDn` by a page less one line, and `Home` `End` jump to either end. It opens at the first match. `n` and `N` move to the next and previous match, wrapping past the last and the first, and the title line shows which one (`match 2 / 3`). `Enter` there opens the resume dialog. `Esc` or `←` returns to the palette with the query and selection unchanged.

Recaps cost nothing extra: Claude Code writes `away_summary` records into its session files. On the real histories, 180 of 783 sessions had one and 100 had a `pr-link` record. Codex and Pi sessions have no recap. Writing a summary for every session would need a model call during indexing; that is deferred (see open questions).

## Defaults and settings

`Ctrl+S` opens settings. Defaults:

| Setting | Default | Reason |
|---|---|---|
| Hide sessions in temp folders (`/private/tmp`, `/tmp`, `/var/folders`, `/private/var/folders`) | On | 59 of 783 real sessions were scratch work in temp folders. The header shows how many are hidden, and `is:temp` shows them anyway. |
| Hide missing folders in the location list | On | 46 of 82 real recorded folders no longer exist. They still appear when the user types a name after `@`. |
| List orphaned sessions after the rest | On | 501 of 783 real sessions were orphaned and crowded out live ones. They stay searchable. |
| Hide orphaned sessions | Off | For users who never want orphans: they are left out of the results and their missing places out of the location list, even after typing past `@`. `is:orphaned` shows them anyway. |
| Accent | Sage (256-color index 108), soft grey selection bar (index 237) | Chosen from cyan, amber, violet, sky, sage, and monochrome. |
| Location icons | Nerd Font | Chosen over plain Unicode (`▣ ⎇ ▢ ⊘`), text tags, and none. |

Terminals cannot report whether a Nerd Font is installed, so the icon set has to stay a setting with a Unicode fallback. `NO_COLOR` switches to the monochrome theme. Selection must remain visible without color (reversed or bold).

The settings screen takes the place of the query line, the results and the preview, inside the same border. It has one row per setting, with its value on the right in the accent:

| Row | Values, in the order `→` steps through them |
|---|---|
| Hide temp-folder sessions | `on`, `off` |
| Hide missing folders in locations | `on`, `off` |
| List orphans last | `on`, `off` |
| Hide orphaned sessions | `off`, `on` |
| Accent | Sage (108, grey bar 237), Cyan (the terminal's cyan, reversed selection), Amber (214), Violet (141), Sky (75), Mono; each but Cyan and Mono has the grey bar |
| Location icons | Nerd Font, Unicode (`▣ ⎇ ▢ ⊘`), Text tags (`git wt dir gone`), None; each shows its four icons so the user can check their font |

Below the rows, `index` and `preferences` name the two files' paths, cut from the left when they don't fit; in a terminal too short for both, they are left out rather than any setting. `↑` `↓` or `Ctrl+P` `Ctrl+N` move between rows, `→` or `Enter` picks the next value, `←` the previous, `Esc` or `Ctrl+S` goes back to the palette, `Ctrl+R` refreshes the index, and `Ctrl+C` quits. Other keys do nothing, so typing never reaches the query. Each change is saved to `prefs.sqlite` and shows at once; the palette searches again from the first row. When it can't be saved, the footer says so and the setting stays as it was.

While temp sessions are hidden, the query line shows `N temp hidden ·` before the match count: how many sessions the query matched that are left out for being in a temp folder. It is dropped first when the query needs the room. Under `NO_COLOR` the accent row reads, for example, `Amber · off: NO_COLOR`: the choice is kept but nothing is drawn in color. With the icon set None, the icon and the space after it are left out, so the text starts where the icon would.

Settings change the palette only; `yapkeeper list` and `search` always use these defaults, so scripts get the same answer whatever the palette is set to.

## Keys

| Key | Action |
|---|---|
| Typing | Insert at the cursor |
| `←` `→` | Move the cursor one character |
| `Alt+←` `Alt+→`, `Alt+B` `Alt+F` | Move the cursor one word |
| `Home` `End`, `Ctrl+A` `Ctrl+E` | Move the cursor to the start or end of the query |
| `Backspace` `Delete` | Delete the character before or after the cursor |
| `Ctrl+W` or `Alt+Backspace`, `Alt+D` | Delete the word before or after the cursor |
| `Ctrl+U`, `Ctrl+K` | Delete to the start or end of the query |
| `↑` `↓`, `Ctrl+P` `Ctrl+N` | Move the selection |
| `Enter` | Resume dialog |
| `Ctrl+O` | Read the whole conversation |
| `Tab` | Location list |
| `Ctrl+L` | Insert the launch directory's `@` token |
| `Ctrl+X` | Hide or unhide the selected session |
| `Ctrl+T` | Include tool calls in search |
| `Ctrl+S` | Settings |
| `Ctrl+R` | Refresh the index now |
| `Esc` | Clear the query, then quit |
| `Ctrl+C` | Quit |

The selection stops at the first and last rows and returns to the first row whenever the query changes.

The query line edits like a one-line text field, with the terminal's own cursor; it shows only there, and not while the location list is open, in the conversation, the settings or the resume dialog. A word is a run of non-space characters, so one word key covers a whole filter such as `@acme-api:billing-retry`. A character is what reads as one: a combining accent or a joined emoji moves and deletes with the character it belongs to, and a wide character takes two cells. Terminals send `Alt` keys either with an `Alt` modifier or as `Esc` before the key, and both work: `Alt+←` may arrive as either, or as `Esc b`, which reads as `Alt+B`. `Ctrl+U` at the end of the query clears it, as it did before the cursor could move. A query wider than its line scrolls sideways to keep the cursor in view, as little as it must, with `…` where text is left out on either side. There is no undo, and the mouse does not place the cursor. Keys that arrive faster than the palette draws, such as a paste, are applied together: the query line is drawn once, and only the last query's results are shown. Results show from the frame after the search finishes, and until then `Enter`, `Ctrl+O`, `Ctrl+X` and the arrows act on the rows on screen.

`Ctrl+R` works on every screen and has no footer hint: the periodic refresh covers the common case, so the hint would only crowd the footer.

While the query line shows, every printable key goes to the query, so all actions use `Enter`, arrows, `Tab`, or `Ctrl` keys. This is why hide has no undo key in the palette: the toast points to `is:hidden` instead.

The resume dialog has no query line, so it takes letters: `Enter` resumes, `c` resumes in the launch directory instead, `r` checks an unavailable folder again, and `Esc` goes back.

## Resume dialog

`Enter` on a result, or in the conversation, opens the resume dialog in place of the query line, results and preview, inside the palette's one border. Its title line reads `Resume "<title>"` with the provider on the right. Below the rule come `directory`, where the provider would start, and `command`, the exact program and arguments the adapter gives, for example `claude --resume <id>` or `codex resume <id> -C <dir>`. Both wrap and are never cut. An argument a shell would split or read as syntax is shown single-quoted; the command itself never goes through a shell. Below them is one row per state that applies, each with a word label, so it reads the same under `NO_COLOR`:

| Label | When |
|---|---|
| `orphaned` | The session's folder is gone, so it can't resume there. While on offer, the row says `c` resumes in the launch directory instead. |
| `unavailable` | The folder exists but can't be inspected (permissions, or a volume not mounted). The row offers `c` as above, and `r` to check again. |
| `orphaned` or `unavailable` | After `c`, the launch directory is gone too, or can't be inspected either. |
| `running` | Claude Code's running-session metadata lists the session as open. Resuming is still allowed. |
| `warning` | After `c`, for a provider that looks sessions up by directory (Claude Code, Pi): it may not find this session from another directory. |
| `not found` | The provider's program isn't on `PATH`, so it can't start. |

Resume never moves to another directory without `c`. `c` is offered while the session's own folder is orphaned or unavailable, and it is one-way: `Esc` and opening the dialog again start from the recorded folder. `r` checks an unavailable folder again and redraws the dialog; when it is still unavailable, the dialog says `Checked again: still can't inspect it.` The result row keeps its label until the next pass. Directory and command come first, so a short terminal drops state rows from the bottom.

A second `Enter` resumes, unless the program isn't found or the directory can't be used; then the dialog's own rows say why and `Enter` does nothing. Yapkeeper restores the terminal, closes the index, and the provider **replaces** Yapkeeper in the same terminal, started with its arguments as separate words and no shell, in that directory. The provider gets the terminal, signals and exit status directly, and nothing of Yapkeeper is left running. If it can't be started, Yapkeeper says why on stderr and exits 1.

`Esc` closes the dialog and returns to the results or the conversation it opened from, unchanged. `Ctrl+C` quits. The footer shows `↵ resume` only while resuming would start, `c here` and `r check again` only while they are offered, then `Esc back` and `^C quit`. Other keys do nothing.

## Loading

`yapkeeper list` reads sessions from the index and reopens only history files whose identity, size or mtime changed since the last run, reading a file that only grew from its last complete line. The palette opens on the sessions already in the index and reads no history file before the first frame. A background pass then reads changed histories in small committed batches, newest first. Each batch's sessions appear in place, keeping the query, the selected session when it is still listed, and the place in an open conversation. Results behind an open conversation or resume dialog update when the user returns to them. `indexing ·` shows dimly before the match count once a pass has run for 150 ms; with an empty index the results read `Indexing histories…`. Searches run on a worker of their own, so typing never waits on one; `searching ·` shows in the same place once a search has run for 150 ms, ahead of `indexing ·`. Quitting or resuming never waits for a search. While the palette is open, a new pass starts 10 s after the last one ended, and `Ctrl+R` starts one at once; pressed during a pass it does nothing, and a file the running pass didn't plan waits for the next one. Each pass, even one that finds no file changed, reads every session's location again, so a deleted or remounted folder shows as `orphaned` or `unavailable`, or stops doing so, after it. A file that grows while it is read is stat'ed before the read, so the next pass reads the rest. Quitting or resuming stops the pass, or the wait for the next one, without waiting: an unfinished batch is discarded, so the index stays as of its last commit. A second Yapkeeper or `yapkeeper list` takes turns with the pass batch by batch, and a pass that finds the index busy for 5 s ends quietly. Unlike the prototype below, the index keeps tool output, capped like tool calls, so `--tools` can search it.

The prototype reads every history file at launch, in parallel threads, without an index. On the real histories (866 files, 1.18 GB) it loaded 783 sessions in about 0.3 s with 55 MB peak memory on the user's Mac. It skips tool results, keeps tool calls as one-line summaries, caps messages at 4,000 characters, and drops injected context such as command caveats, `AGENTS.md` instructions, and environment blocks. Sessions with no user prompt (64) and Codex subagent sessions (19) are left out.

## Where Yapkeeper keeps its files

Both files sit in `$XDG_DATA_HOME/yapkeeper/`, or `~/.local/share/yapkeeper/` when `XDG_DATA_HOME` is unset or relative:

- `index.sqlite`, the index: rebuildable, and safe to delete at any time.
- `prefs.sqlite`, the preferences: hides and settings. Rebuilding the index never touches it. Opening it with an older Yapkeeper than the one that last wrote it fails rather than losing them.

## Command line

| Command | stdout | stderr |
|---|---|---|
| `yapkeeper` | Nothing | On a terminal, opens the palette. Otherwise help, then `yapkeeper: the palette needs a terminal; yapkeeper list and yapkeeper search work in scripts`, exiting 2 |
| `yapkeeper list [--ids]` | One line per session outside temp folders and not hidden, orphaned last, each group most recent first; `--ids` starts each line with the session's id | No sessions found, no sessions outside temp folders (with the `is:temp` hint) or every session hidden (with the `is:hidden` hint), malformed lines skipped |
| `yapkeeper search [--ids] [--tools] <query>…` | Sessions matching every word and filter of the query language, one line each, as `list` | No sessions found, no sessions match (with a `--tools` hint when the query has words), no sessions outside temp folders (with the `is:temp` hint) or every session hidden (with the `is:hidden` hint), unknown provider or state names (with the valid names), malformed lines skipped |
| `yapkeeper paths` | `index` and `preferences`, each followed by its path in a space-aligned column | Nothing |
| `yapkeeper rebuild` | Nothing | `yapkeeper: rebuilt the index at <path> (N sessions)` |
| `yapkeeper hide <id>` | Nothing | `yapkeeper: hid "<title>"; yapkeeper search is:hidden lists it`, or, exiting 1, no session has the id (with the `list --ids` hint) |
| `yapkeeper unhide <id>` | Nothing | `yapkeeper: unhid "<title>"`, or the same error as `hide` |

When the index can't be used or the preferences can't be read or written, any command that uses them exits 1, saying `can't use the index` or `can't use the preferences`. Its stderr ends with `Check its permissions, or set XDG_DATA_HOME.` Hiding a session that is already hidden, or unhiding one that is shown, succeeds and changes nothing.

The help footer names each provider's history directory and the index and preferences paths, as this environment resolves them; when a history directory can't be resolved, it says why instead. `search --help` names every state the query language accepts, and every provider when the history directories resolve.

## Open questions

- Fuzzy matching for search words and locations.
- Summaries for sessions without a recap: an opt-in, cached model call during indexing, which costs tokens and sends conversation text to a model.
- Behavior at 60 columns and 80×24 was built (the preview drops below 80 columns, and a "terminal too small" message appears below 48×12) but not reviewed by the user on real data.
- Plain CLI output mode, `Ctrl+Z` suspend, and mouse support were not prototyped.
