# Yapkeeper

Now, where was I?

Yapkeeper is a local CLI/TUI for finding, browsing, and resuming Claude Code, Codex, and Pi conversations across projects, Git worktrees, and folders, including sessions started outside Yapkeeper.

The project is in planning. `yapkeeper list` prints your Claude Code sessions, most recent first, with title, provider, age and working directory.

On 1.91 GB of local histories, disposable benchmarks indexed in 2.75 seconds with 50 MiB peak RAM in Rust, versus 2.87 seconds and 235 MiB in Bun/TypeScript. We chose Rust with Ratatui for comparable indexing speed and a smaller memory footprint.

- [Product scope and open decisions](docs/product.md)
- [Session-format findings](docs/session-formats.md)
- [Domain glossary](GLOSSARY.md)

Vago builds Yapkeeper with Claude Code and Claude Opus 5.5 by default. Run a Goal from the repository root with `vago run <goal>`. The workflows use Cargo formatting, lint, and test checks, with unsafe Rust forbidden in workspace code; publishing steps still need review before execution.
