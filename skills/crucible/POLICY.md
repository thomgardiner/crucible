# Crucible policy

Confidence does not come from reading every line an agent wrote. It comes from a
**gauntlet of deterministic constraints** the agent cannot quietly walk through.

Many tests may be agent-authored. That is fine. Claiming "tested" or "done" without
running the arms below is not.

## Forbidden claims

- "tested" / "done" / "safe to ship" without the arm evidence for the risk tier
- Editing a gate checker (or judge config) and approving it in the **same commit**
- Waiving a mutant without a provable-equivalence reason in `mutation-waivers.json`
- Treating a custom `--recipe` dry-run as a certification
- Treating a green unit suite as proof the app boots or that tests bite

## Physical constraints beat prose

Rule files and style guides alone do not bind agents. Only gates that **fail the
build** (or `crucible check`) bind. A correctness-critical rule is either:

1. a gate in the required per-change lane, or
2. explicitly **advisory** with a written rationale.

There is no third state.

## Risk-scaled ladder (run before "done")

Stop at the first applicable tier. Record command + exit code + one-line evidence
(or receipt path). Prefer the real binary on PATH.

| If you claim… | You must run | Pass means |
| --- | --- | --- |
| Tests are not hollow | `crucible test-smells <touched test dirs>` | exit 0 |
| Gates are honest | `crucible check` | 0 violations |
| High-risk / money / auth code changed | `crucible cover` then `crucible harden` | no zero-hit high-risk fns; 0 blocking survivors |
| The app works | `crucible run` | verdict RUNS (canonical recipe only certifies) |
| Suite is stable (flaky history) | `crucible flake` | no flip-flops |

**No `.crucible/`?** Run `crucible doctor`. If missing, say "harness not adopted" and
fall back to the repo's own gates — still no tautology tests, still no "done" without
real verification. Do not claim Crucible-grade tested.

## Independence (honest model)

Threat model: single developer + agents that commit as that developer. There is no
second git identity for cryptographic multi-party approval.

What Crucible enforces instead:

1. `adapter.prePush` names a real hook that runs `crucible check` (load-bearing).
2. Approvals are committed **separately** from the config/checker they bless
   (`check` flags same-commit self-approval at HEAD).
3. Non-blank `approvedBy` on every approval record.

## Spot-check (driver / human)

After arms are green, sample 1–3 agent-written tests or acceptance steps for: real
oracle, positive control, capture-grounded fixtures where money is involved. Do not
line-review the whole diff as a substitute for the gauntlet.

## Done report format

```
Arms:
  test-smells: <exit> <paths>
  check: <exit>
  cover/harden: <exit> <survivors or clean>
  run: <exit> <optional; required for app/release claims>

Spot-check: <what>
Advisory / deferred: <what remains unenforced and why>
```

## Non-goals for the agent

- Do not weaken a checker to make check pass.
- Do not delete or no-op a failing proof.
- Do not report "suite green" as evidence of production safety.
