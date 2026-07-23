# Changelog

All notable changes to Crucible are documented here. Crucible follows semantic
versioning.

## Unreleased

### Added

- `harden` / `cover` accept `--candidate` (tip C, default HEAD) with `--base`
  (B) so arms scope to an explicit B..C range instead of a static HEAD-only view.

### Changed

- Config load strips `_…` annotation keys (e.g. `_note`) then refuses any other
  unknown field, so typos cannot silently weaken a gate.
- CI runs the test suite on Ubuntu, macOS, and Windows (format/clippy/self-check
  remain Linux).

## 0.1.0 — 2026-07-23

First public release. A single Rust binary that sits on top of your existing test
tools and fails closed when verification is hollow or incomplete.

### Onboarding

- `crucible init` scaffolds config, a **placeholder** smoke checker (proves wiring,
  not behavior), pre-push, and recipes; sets `core.hooksPath` when unset.
- `crucible doctor` checks PATH, prints next steps, and points at docs/RESOURCES.md.
- [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md) — install → first green check.

### Commands

- `check` / `audit` / `approve` — gate ledger honesty (wiring, approvals, load-bearing pre-push)
- `run` — build, boot, and drive the real app against real oracles
- `harden` — diff-scoped mutation; names survivors as the next tests to write
- `cover` — diff-scoped coverage floor (changed functions never called)
- `flake` — suite determinism across repeated runs
- `test-smells` — hollow tests (no assert, tautologies, silent skips)
- `init` / `doctor` — adopt and health-check `.crucible/`
- `config` — machine-wide concurrency budget for heavy arms

### Behavior

- Fail-closed on empty scope, zero mutants, stale LCOV, invalid patterns, and
  custom `--recipe` dry-runs (no certification receipt).
- Stop nudge clears only on successful `run` / `harden` for the current worktree.
- Pre-push independence: hook must exist and run `crucible check` on an active line.
- Approvals must land in a separate commit from the config they bless.
- Resource bounds for heavy arms (concurrency gate, memory/timeout/output caps).
- Agent skill + cargo-dist installers (shell / PowerShell).

### Proofs

Deterministic contrast suite (`cargo test --test proof`) and detection benchmark
(45/45 planted hacks, 28/28 honest controls). See [docs/PROOFS.md](docs/PROOFS.md).
