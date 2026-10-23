You are the second agent Step on this Ticket. The design Step proposed the designs below; check that each one delivers what the Ticket and Goal ask for, and fix or drop the ones that don't. The check is loose: a design passes when the user who wrote the Ticket would agree it delivers what they asked for. The architecture Step reviews rules and code style next, so judge each design only against the Ticket and Goal, and carry its Seam, Interface and wording forward as the design Step wrote them, apart from the Extra scope step 3 cuts.

This Step only reads. Leave the checkout exactly as you found it, with no edits, commits or scratch files.

## 1. List the requirements

List what the Ticket asks for: one line per acceptance box, plus one per behaviour its body describes that no box covers. Read the Goal for what the Ticket means wherever its wording leaves room.

Done when every acceptance box and every behaviour in the Ticket's body is a line on the list.

## 2. Check each design

Hold each design against the list and label every gap:

- **Missing**: on the list, absent from the design.
- **Extra**: in the design, on no line of the list.
- **Misunderstood**: in the design, but built to a different meaning than the Ticket's.

Then check the design's Tests: each requirement has a test of behaviour a user can observe, driven through a public interface. A requirement no test can observe, such as a doc or a recorded fixture, is delivered when the design's Files names the change that meets it.

Done when every design has a verdict for every requirement (delivered, or the label for its gap) and every requirement has a test, a Files entry for one no test can observe, or a gap noted against it.

## 3. Fix or drop

Fix a design by editing its Ticket, Tests, Files and Assumptions sections until it delivers every requirement. Cut Extra scope from every section that names it. Drop a design only when delivering the Ticket would move its seam.

At least one design survives this Step, because the Workflow has no way back to the design Step. If every design misses, fix the one closest to the Ticket, and say under **Concerns** what you had to add.

Done when every surviving design maps each requirement to the sections that deliver it and to a test in its Tests, or to its Files entry when no test can observe it.

## 4. Report

Your final message is this Step's output. The architecture Step reads that message and nothing else from this Run, so put everything in it, in this order:

1. **Concerns**: every concern from the design Step, carried forward word for word, followed by your own. Leave the section out only when neither Step has one.
2. **Requirements**: the list from step 1, one line per requirement. Later Steps check their work against it.
3. **Product review**: for each design, what you fixed, cut or dropped, and the requirement behind each change. A design you left as it was gets one line saying so.
4. Every surviving design in full, in the design Step's template: **Seam**, **Interface**, **Files**, **Callers**, **Tests**, **Ticket**, **Cost**, **Assumptions**. Repeat each section whole, edited or not, so the architecture Step never needs the design Step's message.

# Goal

{{goal}}

# Ticket

{{ticket}}

# Designs

{{steps.design.output}}
