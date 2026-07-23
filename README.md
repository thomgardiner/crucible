# Crucible

Crucible sits on top of the tests you already run. It does not replace `cargo test`,
Vitest, or Playwright. It answers a narrower question: **did this change get verified
in ways a green unit suite alone can miss?**

That matters when most of the code (and many of the tests) come from coding agents.
A suite can pass while asserting nothing useful, while the app fails to boot, or while
a gate that should have blocked the change was never wired.

## What it checks

| Command | Question |
| --- | --- |
| `crucible check` | Are the project’s gates still honest — wired, approved, not quietly weakened? |
| `crucible run` | Does the real app build, boot, and pass its drive oracles? |
| `crucible harden` | Do the tests constrain the changed code? (diff-scoped mutation) |
| `crucible cover` | Were the changed functions executed at all? |
| `crucible flake` | Is the suite deterministic across identical runs? |
| `crucible test-smells` | Are there hollow tests (no assert, tautologies, silent skips)? |

Use the arms that match the risk of the change. A docs-only edit is not a checkout
path; money and auth code should not ship on “tests passed” alone.

## Why this helps

These failures are common and easy to miss in a normal green CI:

- Tests that run code but never assert (or only `assert!(true)`).
- A green suite while the app panics on startup.
- A never-called function in high-risk code that no test touches.
- A gate weakened or left unwired so the rule never fires.
- A “verified” claim based only on gate wiring, not a real test or boot.

Crucible fails closed on those cases. The claims are exercised on every
`cargo test --test proof` — short table and how to reproduce them live in
[docs/PROOFS.md](docs/PROOFS.md).

## Install

**CLI (required):**

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy ByPass -c \
  "irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex"
```

```sh
# From a source checkout
cargo install --locked --path .
crucible --version
```

**Optional:** install this repo as a Claude Code / Codex plugin for the skill and
Stop nudge (`after-install.md` after install). The CLI is still required on PATH.

## Setup (one repo)

```sh
cd your-project
crucible init                 # scaffold + smoke gate + pre-push; sets hooksPath if unset
crucible approve smoke --by "$USER"    # gates first (rewrites charter)
crucible approve __config__ --by "$USER"
# Commit approvals separately from the config files.
crucible doctor && crucible check
```

Then fill recipe TODOs in `.crucible/acceptance.json` (and mutation/coverage/flake
as needed) and set `highRiskUnits`. Full walkthrough:
[docs/GETTING_STARTED.md](docs/GETTING_STARTED.md).

## Day-to-day

```sh
crucible test-smells path/to/tests
crucible check
crucible run          # needs acceptance.json filled
crucible cover        # needs coverage recipe + a dirty diff
crucible harden       # needs mutation recipe + a dirty diff
```

`harden` and `cover` are **change-scoped**. On a clean tree they refuse to certify
(nothing to measure) — intentional.

## Agents

A Claude Code / Codex skill ships under `skills/crucible/`. It steers agents to run
the same CLI before calling a change “tested.” CI and pre-push remain the backstop.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

This repository uses Crucible on itself (`.crucible/`). Heavy arms are concurrency-
and memory-bounded; details: [docs/RESOURCES.md](docs/RESOURCES.md).
