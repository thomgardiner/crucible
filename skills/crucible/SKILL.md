---
name: crucible
description: Keep yourself honest about testing before reporting a coding task done. Prove the gates are real (crucible check), the app is real (crucible run), and the tests actually bite (crucible harden). Use before claiming code is tested, when a suite is green but the app breaks, before finishing work on money/checkout code, or when adopting Crucible in a repo.
---

# Crucible

A green test suite and a working app are different claims. In agent-authored code they
diverge fast: it is easy to write a test that runs the code and asserts nothing, or one
that mocks the exact seam a real launch crosses, then report "tested." Crucible is the
honesty layer that catches that. It does not run your tests or replace Playwright /
Vitest / `cargo test`. It sits on top of whatever you used and checks that "tested" is real.

`crucible` is a CLI. Install it so it is on PATH (see the repo README:
`cargo install --locked --path .`, or the curl/powershell installer). This skill is the
inner-loop on-ramp; the same commands run in CI as the backstop that does not depend on
an agent choosing to check itself.

## The one law

A correctness-critical rule is a gate in the required per-change lane, or it is
explicitly declared advisory with a written rationale. There is no third state.
"Written in the rules doc" is not enforcement.

## Run it before you report a task done

Before you say a change is tested or an app works, run the arm that proves it:

- **Before claiming the app works** run `crucible run`. It builds, boots, and drives the
  real artifact. A green unit suite says nothing about whether the app comes up.
- **Before finishing a change to money/checkout code** run `crucible harden`. A surviving
  mutant on changed code is proof no test constrains that behavior; it names the exact
  test to write, so write it and rerun.
- **To catch functions no test calls at all** run `crucible cover`. Diff-scoped coverage
  names changed functions never executed by any test — the floor under harden (coverage =
  was it run, mutation = was it checked).
- **Before trusting a gate** run `crucible check` (and `crucible audit` for the map). It
  fails if a declared gate is not wired, runs unregistered, or had its oracle or config
  changed without an independent approval.

Also useful: `crucible test-smells <dir>` screens for the laziest hacks (skipped tests,
focused tests, assertion-free bodies, tautological asserts) and `crucible doctor` reports
whether `.crucible/` is wired right.

## The three arms

| Command | Proves | Blocks when |
| --- | --- | --- |
| `crucible check` | the **gates** are real | a declared gate is not wired where it claims, a checker runs unregistered, or an oracle changed without an independent approval |
| `crucible run` | the **app** is real | the real artifact does not build, boot, or pass its drive oracles |
| `crucible harden` | the **tests** are real | a mutant survives on changed high-risk code (no test constrains that behavior) |

The recipe for `run` lives in `.crucible/acceptance.json` (build, boot with a readiness
oracle, drive with real oracles, plus a trust audit of mock-boundary test files). The
mutation command and diff base for `harden` live in `.crucible/mutation.json`; survivors
land in `.crucible/survivors.json` as the next tests to write.

## Configure the machine (once per machine)

The heavy arms (`run`, `harden`, `cover`, `flake`) are serialized machine-wide by
default — one build/test tree at a time — so parallel sessions queue instead of
collectively OOM-ing the box. Part of setting Crucible up is deciding that budget:

```
crucible config                      # show the live slot count, its source, and the per-tree memory ceiling
crucible config max-concurrency 2    # persist N slots (machine-level file, capped at core count)
```

More slots means more parallel heavy runs but a lower memory ceiling per tree (the
80%-of-RAM budget splits N ways); the command prints the resulting ceiling when you set
it. `CRUCIBLE_MAX_CONCURRENCY` overrides per-shell. Do not raise the count just because
your run is queued behind another session — the queue is the protection.

## Adopting in a repo

```
crucible init                       # scaffold .crucible/ (adapter, charter, recipes, approvals)
# fill the TODOs: gate runner + highRiskUnits (adapter), build/boot/drive (acceptance), mutation cmd
crucible approve __config__ --by <reviewer>   # a reviewer, not the author, pins the config
crucible approve <gate> --by <reviewer>       # ...and each gate's oracle
crucible doctor                     # is it wired right?
crucible check                      # confirm every gate is honest
```

The adapter is the only per-repo binding: it names the required gate runner, the
high-risk units, and the pre-push hook. The approver must not be the author: commit an
approval separately from the change it approves, and require a fresh `crucible check` in
the pre-push hook. Full contract: `docs/ADOPTING.md`. Grounding and proofs:
`docs/METHODOLOGY.md`, `docs/PROOFS.md`, `docs/POSITIONING.md`.
