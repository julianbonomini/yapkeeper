# Yapkeeper

Now, where was I?

Yapkeeper is a local CLI/TUI for finding, browsing, and resuming Claude Code and Codex conversations across projects, Git worktrees, and folders, including sessions started outside Yapkeeper.

The project is in planning. The `yapkeeper` command is not implemented yet.

On 1.91 GB of local histories, disposable benchmarks indexed in 2.75 seconds with 50 MiB peak RAM in Rust, versus 2.87 seconds and 235 MiB in Bun/TypeScript. We chose Rust with Ratatui for comparable indexing speed and a smaller memory footprint.

- [Product scope and open decisions](docs/product.md)
- [Session-format findings](docs/session-formats.md)
- [Domain glossary](GLOSSARY.md)

Vago uses Codex with GPT-6.1 Sol, high reasoning, and Fast mode by default. A local planning item named `yapkeeper-v1` uses the Codex harness. Open its planning session from the repository root:

```sh
vago plan session yapkeeper-v1
```

Planning items stay in this checkout. The workflows use Cargo formatting, lint, and test checks, with unsafe Rust forbidden in workspace code; publishing steps still need review before execution.
