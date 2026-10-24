# Yapkeeper

Now, where was I?

Yapkeeper is a local CLI/TUI for finding, browsing, and resuming Claude Code, Codex, and Pi conversations across projects, Git worktrees, and folders, including sessions started outside Yapkeeper.

Run `yapkeeper` in a terminal to open the palette: type to search your Claude Code, Codex and Pi sessions, preview one, and press `Enter` twice to resume it in its own directory. `yapkeeper list` and `yapkeeper search` print the same sessions one line each for scripts, and `yapkeeper --help` covers the query language and where Yapkeeper keeps its files.

On 1.91 GB of local histories, disposable benchmarks indexed in 2.75 seconds with 50 MiB peak RAM in Rust, versus 2.87 seconds and 235 MiB in Bun/TypeScript. We chose Rust with Ratatui for comparable indexing speed and a smaller memory footprint.

- [Product scope and open decisions](docs/product.md)
- [Session-format findings](docs/session-formats.md)
- [Domain glossary](GLOSSARY.md)

Vago builds Yapkeeper with Claude Code and Claude Opus 5.5 by default. Run a Goal from the repository root with `vago run <goal>`. The workflows use Cargo formatting, lint, and test checks, with unsafe Rust forbidden in workspace code; publishing steps still need review before execution.

## Measuring launch and search

`yapkeeper-bench` measures how fast the palette opens over a warm index and how fast it answers each keystroke of a query and a pasted query, against the 200 ms and 50 ms targets in [docs/product.md](docs/product.md#speed-and-refresh). It writes a synthetic corpus from a seed, indexes it into a temporary folder, and prints p50 and p95 for each measurement on stdout. Run it from the repository root in a release build:

```sh
cargo run --release -p yapkeeper-bench
```

The corpus takes about 1 GB in a hidden folder inside `--dir` (default: the current directory) while it runs and is deleted at the end. `--dir` must not be under a system temp folder such as `/tmp`, where every session would count as temp. `--seed`, `--sessions`, `--subagents`, `--megabytes` and `--runs` change what is measured; `--help` lists them. The checks don't run it.

The benchmark reads your own histories only with `--real-histories`, and only with your authorization. It reads the Claude Code, Codex and Pi directories this environment names without writing to them, builds a temporary private index that is deleted at the end, and prints counts and timings only. Keep its output out of commits and pull requests.

## Releasing

Releases are cut from the version in the root `Cargo.toml`. Bump `version` under `[workspace.package]` and merge to `main`. The release workflow sees a version with no tag, runs the checks, builds macOS (Apple Silicon and Intel) and static Linux (x86_64 and arm64) binaries, and publishes a GitHub Release tagged `v<version>` with tarballs and SHA-256 checksums. A push that leaves the version unchanged releases nothing.
