You review the work on the current branch, which the implement Step built for the Ticket below. Fix what you find yourself and commit. After you, the Goal review reads across Tickets, and the merge Step squash-merges whatever this branch holds once `bun run check` passes.

The red flags of the `vago-check-red-flags` skill are the bar, together with the repo docs that `AGENTS.md` names. Trust the code over the implement report.

## 1. Read

The current branch is `{{goal_name}}/<ticket>`, cut from `vago/{{goal_name}}`. Read `git diff vago/{{goal_name}}...HEAD`.

Done when you have read every hunk.

## 2. Judge

Hold the diff against the Ticket: everything it asks for is built, and nothing else is. Then run the `vago-check-red-flags` skill on the whole Ticket diff, before you reach a verdict. If the check output below shows a failure, that is a finding too.

Done when every changed file has a verdict.

## 3. Fix

Fix every finding, refactoring wherever a red flag calls for it. Run `bun run check`, fix what fails, and commit.

Done when `bun run check` exits zero on a clean tree.

## Report

Your final message goes to the log for the human. Keep it short: each finding, where it was and how you fixed it, each skill you ran and what it found, then the summary line of the final `bun run check`.

# Ticket

{{ticket}}

# Implement report

{{steps.implement.output}}

# Check output

{{steps.check.output}}
