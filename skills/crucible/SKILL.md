---
name: crucible
description: >
  Honesty harness for agent-written code. Surrounds agents with deterministic gates
  so "tested" means something. Use before reporting any coding task done, when a suite
  is green but the app breaks, on money/checkout changes, before release, or when
  adopting Crucible. Triggers: /crucible, "is this tested", "green suite", "ready to
  ship", "mutation", "reward hack", "harden".
---

# Crucible

Read `POLICY.md` in this skill directory completely and follow it.

## CLI

Install so `crucible` is on PATH (`cargo install --locked --path .`, or the release
installer). This skill is the inner-loop on-ramp; CI/pre-push run the same commands
as a backstop that does not depend on the agent choosing to check itself.

## Quick map

| Command | Proves |
| --- | --- |
| `crucible check` | gates are real (wired + approved; pre-push load-bearing) |
| `crucible run` | app is real (build/boot/drive; only canonical recipe certifies) |
| `crucible harden` | tests bite (diff-scoped mutation) |
| `crucible cover` | changed high-risk code was executed |
| `crucible test-smells` | cheapest reward hacks (tautologies, empty tests, skips) |
| `crucible doctor` | `.crucible/` is wired |
| `crucible flake` | suite is deterministic |

Full procedure, forbidden claims, risk ladder, and done-report format: **POLICY.md**.
Adoption: repo `docs/ADOPTING.md`. What the product proves: `docs/PROOFS.md`.
