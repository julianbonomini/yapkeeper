# PROTOTYPE: Yapkeeper command palette

Disposable. It settles the interaction model in `docs/product.md` → *UI prototype questions* before implementation tickets are written. Resume is simulated: after you confirm, the TUI exits and prints the command it would have run.

```sh
cargo run --release --manifest-path prototypes/tui-flow/Cargo.toml              # your real histories
cargo run --release --manifest-path prototypes/tui-flow/Cargo.toml -- --demo    # synthetic sessions
cargo run --release --manifest-path prototypes/tui-flow/Cargo.toml -- --stats   # aggregate counts only
```

The real mode reads `~/.claude/projects` and `~/.codex` (or `CLAUDE_CONFIG_DIR` / `CODEX_HOME`) read-only at launch and keeps everything in memory. It writes nothing, and hiding lasts only until you quit. Tool results are skipped, tool calls are kept as short one-liners, and messages are capped at 4,000 characters.

Type to search everything. `Tab` browses locations (`@project` / `@project:worktree`), `^L` inserts the launch location, `#codex` / `#claude` filter by agent, and `is:orphaned` / `is:unavailable` / `is:hidden` filter by state. `Enter` resumes (simulated), `→` reads the whole conversation, `^X` hides, `^T` includes tool calls, and `Esc` clears then quits.

## Query language

Words search conversations, titles, and recaps. Everything else is a filter, and filters combine with AND.

| Form | Meaning |
|---|---|
| `@where` | location: project, `project:worktree`, or any part of a path |
| `#agent` | `#claude`, `#codex` |
| `is:state` | `orphaned`, `temp`, `git`, `folder`, `pr`, `recap`, `unavailable`, `hidden` |
| `!x` / `not:x` | exclude: `!codex`, `!temp`, `!@scratch`, `!is:pr`, `not:orphaned`; `!word` excludes sessions containing that word |

`@`, `#`, `is:`, `!`, and `not:` each open a completion list. A space never triggers anything, so `is pr` is plain text. Hidden sessions appear only with `is:hidden`.

`^S` opens settings: hide temp-folder sessions (default on), hide missing folders from the location list (default on), list orphans last (default on), accent theme, and location icon set (Unicode, Nerd Font, text tags, none). Settings last only until you quit.

## Later

- Move the cursor inside the query (←/→, word jumps) so a filter in the middle can be edited without deleting what follows.
