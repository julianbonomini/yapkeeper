You build one Ticket of the Goal below, on the branch that is already checked out. A fresh agent reviews your work after you.

For terminal UI work, load `tui-design` and its Rust reference before proceeding. Follow `AGENTS.md` for conditional use of `emil-design-eng`; report the concrete UI decisions those skills informed.

Read `AGENTS.md` before editing code and follow its safe-Rust and crate-root lint requirements.

Run the `vago-write-code` skill while you write code. It is the bar for every line, together with the repo docs that `AGENTS.md` names. A deep, simple design beats a quick one.

## 1. Plan

Read the Ticket, the Goal and the code the Ticket touches. Decide which module owns the change and what its interface is.

Done when you can name each file you will change and why it is the owner.

## 2. Build

Build test-first, one behaviour at a time: write a test, watch it go red, write the least code that turns it green, commit. Each test drives a public interface. Write the fewest tests that pin what the Ticket asks for.

Done when everything the Ticket asks for has a green test, or, where no test can see it (a doc, a comment), a change you can name.

## 3. Check

Before the final check, run the `vago-check-red-flags` skill on the whole diff since `vago/{{goal_name}}`, fix every finding and commit. Then run `sh .vago/scripts/check.sh`, fix what fails and commit.

Done when `sh .vago/scripts/check.sh` exits zero on a clean tree.

Commit on this branch with short imperative messages. The Workflow's shell Steps switch branches, merge and push.

## Report

Your final message is all the review Step reads from you. Keep it short:

- **Built**: what you changed, file by file.
- **Choices**: each question the Ticket left open, and what you picked.
- **Skills**: each skill you ran, and what it found or fixed.
- **Check**: the summary line of the final `sh .vago/scripts/check.sh`.

# Goal

{{goal}}

# Ticket

{{ticket}}
