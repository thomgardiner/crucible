# Agent guide

<!-- crucible:agents:v1 -->
## Crucible: verification honesty contract

This repository keeps testing honest with Crucible. Before you report a change
as tested or done, verify it with the CLI:

- `crucible run` builds, boots, and drives the real app. A green unit suite is
  not proof the app comes up.
- `crucible harden` runs diff-scoped mutation testing. A surviving mutant on
  changed code means no test constrains that behavior; write the test it names.
- `crucible check` confirms the gates are real and no oracle was weakened
  without an independent approval.

Run mutation work through `crucible harden`, not a parallel raw `cargo mutants`
command. Harden applies the recipe's aggregate process-tree memory ceiling;
bypassing it also bypasses that safety boundary.

Do not weaken a checker, waive a mutant without a provable-equivalence reason,
or report work as tested when these are not clean. Full guidance:
`skills/crucible/SKILL.md`.
