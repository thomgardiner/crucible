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

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy ByPass -c \
  "irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex"
```

Or from a checkout: `cargo install --locked --path .`

## Setup (one repo)

```sh
crucible init
# Fill TODOs in .crucible/: gate runner, highRiskUnits, build/boot/drive, mutation cmd
crucible approve __config__ --by <reviewer>
crucible approve <gate> --by <reviewer>
git config core.hooksPath .githooks   # so pre-push runs crucible check
crucible doctor
crucible check
```

Commit each approval in a **separate** commit from the config it blesses.
`init` is idempotent and will not overwrite existing files unless you pass `--force`.

Adoption details: [docs/ADOPTING.md](docs/ADOPTING.md).

## Day-to-day

```sh
crucible test-smells path/to/tests   # hollow tests?
crucible check                       # gates still honest?
crucible run                         # app still boots?
crucible cover                       # changed code executed?
crucible harden                      # tests still bite on the diff?
```

`harden` and `cover` are **change-scoped**. On a clean tree they refuse to certify
(nothing to measure) — that is intentional.

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
