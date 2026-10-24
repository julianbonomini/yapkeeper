# Yapkeeper

Now, where was I?

![Yapkeeper narrowing to a project, searching a conversation, reading it and resuming it](docs/demo.gif)

Yapkeeper finds your Claude Code, Codex and Pi sessions across projects, Git worktrees and plain folders, and resumes them in the directory they ran in. It reads every session in your history, including ones you started without it.

## Install

Yapkeeper runs on macOS and Linux.

With [Homebrew](https://brew.sh):

```sh
brew install julianbonomini/tap/yapkeeper
```

With the install script:

```sh
curl -fsSL https://raw.githubusercontent.com/julianbonomini/yapkeeper/main/install.sh | sh
```

The script downloads the release for your OS and CPU, checks its SHA-256 checksum, and puts the binary in `~/.local/bin`. Set `YAPKEEPER_INSTALL_DIR` to install it somewhere else, or `YAPKEEPER_VERSION=v0.1.1` to pin a release.

You can also download a binary from the [releases page](https://github.com/julianbonomini/yapkeeper/releases) (Apple Silicon and Intel Macs, static Linux builds for x86_64 and arm64), or build from source with a recent stable Rust:

```sh
cargo install --locked --git https://github.com/julianbonomini/yapkeeper yapkeeper
```

## Usage

Run `yapkeeper` to open the palette. Type to search, move with the arrow keys, and press `Enter` twice to resume the selected session in its own directory. On terminals at least 80 columns wide a preview of the conversation sits beside the results; `Ctrl+O` opens the whole conversation at any width, `Tab` browses sessions by location, and `Ctrl+S` opens settings.

A query is a list of words and filters, and a session has to match all of them:

```text
billing retry            words in the title, messages or recap
@yapkeeper               a project, project:worktree or any part of the path
#codex                   one provider: #claude, #codex or #pi
is:worktree              a state, such as is:git, is:pr, is:orphaned or is:hidden
!temp  not:flaky         leave out a filter, state or word
```

Typing `@`, `#`, `is:` or `!` in the palette opens a list of completions with session counts. A word with an uppercase letter matches case. Tool calls, tool output and subagent transcripts are searched only after `Ctrl+T`.

The same search works from the shell, one session per line:

```sh
yapkeeper list
yapkeeper search '#claude' is:worktree flaky test
yapkeeper search --ids '!temp' release
yapkeeper hide <id>
```

`yapkeeper --help` and `yapkeeper search --help` cover the rest.

Yapkeeper never writes to your session histories. It reads them from `~/.claude/projects`, `~/.codex` and `~/.pi/agent/sessions` (Claude Code and Codex honor `CLAUDE_CONFIG_DIR` and `CODEX_HOME`) and keeps a search index and your preferences in its own folder. `yapkeeper paths` prints where. The index can always be rebuilt with `yapkeeper rebuild`; hidden sessions and settings live in the preferences file and survive a rebuild.

## Contributing

Issues and pull requests are welcome. For anything beyond a small fix, open an issue first so we can agree on the behavior.

Yapkeeper is a Rust workspace built on [Ratatui](https://ratatui.rs) and SQLite. Before sending a change, run:

```sh
sh .vago/scripts/check.sh
```

It checks formatting, runs Clippy with warnings as errors, and runs the tests. CI runs the same script on Linux and macOS for every pull request. Unsafe code is forbidden in every crate.

Tests must use synthetic or redacted session data. Don't commit real transcripts, titles or project paths, and don't paste them into issues either.

Some background before you change behavior:

- [docs/product.md](docs/product.md): what Yapkeeper does and the speed targets
- [docs/interface.md](docs/interface.md): the palette's layout and keys
- [docs/session-formats.md](docs/session-formats.md): how each provider stores and resumes sessions
- [GLOSSARY.md](GLOSSARY.md) and [docs/adr/](docs/adr/): terms and past design decisions

`cargo run --release -p yapkeeper-bench` measures launch and per-keystroke search time against those targets on a generated corpus of about 1 GB; `--help` lists its options.

### Releasing

Bump `version` under `[workspace.package]` in the root `Cargo.toml`, run `cargo check` so `Cargo.lock` picks it up, and merge to `main`. CI notices the new version, runs the checks, builds the binaries, publishes a GitHub release tagged `v<version>`, and updates the formula in [homebrew-tap](https://github.com/julianbonomini/homebrew-tap).

## License

[MIT](LICENSE)
