# Changelog

## Unreleased

- **Skill POLICY:** `skills/crucible/POLICY.md` is the agent gauntlet (forbidden claims,
  risk ladder, independence honesty, done-report). SKILL.md is thin and points at it.
- **`.gitignore`:** excludes local agent harness dirs (`.claude/`, `.grok/`,
  `.agents/`, …), internal plan docs, and mutation/coverage noise; keeps product
  plugin manifests (`.claude-plugin/`, `.codex-plugin/`).
- **Meta-proof honesty:** proof suite asserts material side-effects (receipts,
  survivors.json, exit codes), not banners alone; positive controls for clean
  paths; self `test-smells` on `tests/` + `src/`; custom-recipe proof requires
  receipt absence + canonical path presence. PROOFS.md documents the rule.
- **Independence honesty (single-dev + agents threat model):** cryptographic
  “approver ≠ author” is impossible when agents commit as the developer. Instead:
  - `adapter.prePush` is **load-bearing** — `check`/`doctor` fail if it is missing,
    the file is gone, or the hook does not run `crucible check` on an active line.
  - `init` scaffolds `.githooks/pre-push` that invokes `crucible check`.
  - Same-commit self-approval: when **HEAD** last wrote the approvals log together
    with judge config, `check` fails (historical monorepo dumps that co-committed
    once no longer fail forever after HEAD moves on).
  - Docs (POSITIONING, ADOPTING) state the claim plainly: independence = verified
    pre-push wiring + auditable separate-commit trail, not multi-party crypto.
- `harden` / `cover` / `run`: a custom `--recipe` is a **dry run** — never certifies
  (no receipt); only the repo’s approved `.crucible` recipe can mint evidence.
- Fail-closed diff scoping: empty high-risk / empty scope cannot certify; high-risk
  units match path components / file stems, not bare substrings.
- `crucible config max-concurrency <N>`: persist the machine-wide concurrency budget to
  a machine-level config file (env var > file > default, always capped at core count).
  `crucible config` and `crucible doctor` report the effective value and which layer set
  it, plus the resulting per-tree memory ceiling. Ships with a `/crucible-config` command
  and a setup step in the skill.

- Two more arms: `crucible cover` (diff-scoped coverage floor — names changed functions no
  test calls) and `crucible flake` (runs the suite N times and flags nondeterminism).
- Structured artifacts: `run --json`, and each arm fails closed (a killed, timed-out, or
  no-evidence run never reads as a pass).
- Resource safety for the four arms that spawn a build/test tree:
  - Machine-wide concurrency gate (`CRUCIBLE_MAX_CONCURRENCY`, default 1) so parallel
    sessions cannot collectively exhaust memory.
  - Kernel-enforced containment on Linux via `systemd-run --user --scope`: `MemoryMax`,
    `MemorySwapMax=0`, `TasksMax`, and cgroup teardown that reaches `setsid` escapes.
  - Fallback on other hosts: polled memory ceiling (machine-aware default), output/disk
    cap, hard timeout, whole-process-group kill, and reap-on-signal so a killed session
    does not orphan its build tree. `crucible doctor` reports which path is active.
  - Pre-slot diff-discovery (`harden`/`cover`'s `git diff`/`ls-files`) now runs under a
    60-second hard timeout with a process-group kill and an output cap, so a hung or
    flooding git cannot hang Crucible or fill the disk outside the gate.
- Detection benchmark (`cargo test --test benchmark`) with a labeled hack/honest corpus,
  and the judgment core mutation-tested against its own suite.

## 0.1.0

Initial release. A single Rust binary with three arms:

- `crucible check` / `audit` / `approve`: the gate ledger and its honesty checker, with
  oracle pinning and independent-approval provenance (approver is not the author).
- `crucible run`: builds, boots, and drives the real artifact against real oracles, plus
  a static trust audit of mock-boundary test files.
- `crucible harden`: diff-scoped mutation gate, blocking on high-risk (money/checkout)
  units and advisory elsewhere, emitting each survivor as the next test to write.
- `crucible init` / `doctor`: idempotent `.crucible/` scaffolding and adoption health.
- `crucible test-smells`: the shipped test-gaming checker for Rust and TS/JS.

Distributed via cargo-dist (shell and PowerShell installers).
