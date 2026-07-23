---
id: kb-crucible-positioning
type: reference
description: What Crucible is, who it is for, and how to talk about it. Source of truth for README and landing copy.
---

# Positioning

## One line

**Crucible keeps coding agents honest about testing.** It does not run your tests
and it does not replace Playwright, Vitest, or `cargo test`. It sits on top of
whatever the agent used and refuses to let the agent claim "tested" when it isn't.

Tagline candidates (pick one, use it everywhere):

- The honesty layer for agent-written tests.
- Agents will tell you it's tested. Crucible checks.
- Your agent's tests pass. Crucible proves they bite.

## Who it is for

People shipping code written mostly by coding agents (Claude Code, Codex, Cursor).
That audience has one specific, growing pain: **the agent writes tests that pass
without proving anything, then reports "done."** Crucible is built for them, not for
QA teams choosing a test framework.

## The enemy

One concrete failure, three faces:

- A test that runs the code and **asserts nothing** (or asserts a tautology).
- A **green suite over a broken app** — every unit test mocks the one seam a real
  boot crosses, so the app crashes on launch while the number stays green.
- A **gamed gate** — the agent weakens the checker, or self-approves, so the thing
  that would have caught it never fires.

The agent that does any of these and writes "✅ tested" is who Crucible exists to stop.

## What Crucible is NOT

- **Not a test runner.** It orchestrates the tools you already have. You bring
  Playwright / Vitest / `cargo test`; Crucible judges whether the agent used them
  honestly.
- **Not a coverage tool.** Coverage says what ran. Crucible says whether it was
  actually verified.
- **Not a smoke-test framework.** `run` uses your real build/boot/drive commands to
  make the agent's own claimed testing touch the real thing. It is a check on
  honesty, not a competing e2e product.

## The three levels of cheating it closes

An agent can cheat at three levels. Crucible is the only thing that closes all three.

| Arm | The cheat it catches |
| --- | --- |
| `harden` (mutation) + `test-smells` | tests that run the code but assert nothing |
| `run` (reality) | the green suite over a broken app |
| `check` (honesty) | gaming the gate itself: a weakened checker or a faked approval |

The pitch is not "mutation + smoke + gates." It is: an agent can cheat at the test,
the app, and the gate, and Crucible closes all three. No single-purpose competitor
can copy that without becoming Crucible.

## The proof claim

"Anti-reward-hacking" is a falsifiable claim, not a vibe, so it has to be proven.
Crucible ships deterministic proofs that it catches a faked test, a faked gate, and a
broken app, reproduced on every run. Lead with an empirical "catches N of N planted
hacks" number. This audience is academic-adjacent (SpecBench, Meta ACH) and trusts
evidence over adjectives.

## Scope honesty

Crucible catches cheats at the test, app, and gate level. It does not read the agent's
reasoning (no chain-of-thought monitor). Say so plainly. Overclaiming "we stop all
reward hacking" is the one thing that gets this dunked on, and precise scope is more
convincing to this audience than a bigger claim.

**Each arm is only as strong as the recipe the repo provides, and it must fail closed.**
`run` grades a repo-authored acceptance recipe, `harden` a repo-authored mutation command,
`cover` a repo-authored coverage command. Crucible does not independently validate that a
recipe is honest — so an empty, absent, no-oracle, or unfinished recipe must be a hard
error, never a pass, and the CLI enforces that: `run` rejects a missing/empty command, an
oracle that does not compile, or one that passes on empty output; `harden` rejects a run
with no completion evidence; `cover` and `flake` reject a timeout, a non-zero exit, or an
invalid pattern. "Objective" means the evidence is a real artifact (a surviving mutant, a
zero-hit function, a boot crash), not that the checks are unbypassable by a hostile config.

Two limits are by design, not bugs, and are stated rather than hidden: `check` verifies a
gate's wiring and approval **statically** — it does not execute the checker, so it proves a
checker is registered and its bytes are approved, not that it ran and blocked at runtime;
and approver independence (approver ≠ author) is enforced at the repo's pre-push layer, not
by the approval record, which the core only requires to be present and non-blank. A
third-party review (Codex) found several exit-0 bypasses in the recipe-grading arms; those
are fail-closed and guarded by regression tests. The honest framing is "strong, fail-closed
signals," not "un-gameable."

## Delivery: skill on-ramp, CLI backstop

Crucible ships in two layers, and both matter:

- **The skill** (`skills/crucible/`) is the on-ramp. It makes an agent run `crucible run`
  / `harden` / `check` on itself at the right moments, in the inner loop.
- **The CLI** is the engine and the backstop. It runs the same checks in CI and pre-push,
  where enforcement does not depend on the agent choosing to check itself.

Positioning purely as "a skill" drops the enforcement half: a reward-hacking agent will
skip the check that would catch it, so the skill alone cannot be the net. Say "ships as a
skill for the agent loop, and a CLI for the CI backstop." Both install without the
internal dev-team plugin: the CLI via its installer, the skill from Crucible's own repo.

## Messaging rules

- Enemy first, then the positive promise: "proves your agent's code works, and its
  tests actually bite."
- Never say "test suite" for Crucible itself. It is a gate, a harness, an honesty
  layer. It judges suites; it is not one.
- Do not benchmark against Playwright/Vitest/coverage as competitors. They are what
  the agent uses; Crucible sits above them.
- Keep the on-ramp frictionless (zero-config `run`, a GitHub Action, a badge) so the
  sharp message lands on a door people can open.
