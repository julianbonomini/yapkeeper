# Yapkeeper

Now, where was I?

Yapkeeper is a local CLI/TUI for finding, browsing, and resuming Claude Code and Codex conversations across projects, Git worktrees, and folders, including sessions started outside Yapkeeper.

The project is in planning. The `yapkeeper` command is not implemented yet.

- [Product scope and open decisions](docs/product.md)
- [Session-format findings](docs/session-formats.md)
- [Rust, Go, and TypeScript performance comparison](docs/language-performance.md)
- [Benchmarks on actual local histories](docs/real-history-benchmarks.md)
- [Domain glossary](GLOSSARY.md)

Vago is initialized with its starter workflows. A local planning item named `yapkeeper-v1` uses the Codex harness. Open its planning session from the repository root:

```sh
vago plan session yapkeeper-v1
```

Planning items stay in this checkout. The starter workflows have not been run; their runtime, model, checks, and publishing steps need review before implementation starts.
