# Crucible

**The brutal AI testing framework.** In a codebase where LLM agents write most of the
code, the test suite goes green while proving nothing, and the app crashes on launch
anyway because thousands of unit tests mock the exact seam a real boot crosses. Crucible
replaces trust with three checks, plus one law.

- **`crucible check`** verifies the **gates** are honest: every load-bearing rule is
  wired into the required lane, no checker runs unregistered, and no gate's checker or
  config was weakened without an approval recorded separately from the change. It checks
  wiring and approval integrity statically (it does not execute the checkers). Independence
  is **not** multi-party cryptography: under a single-developer + agents model, Crucible
  verifies that `prePush` actually runs `crucible check` and flags same-commit self-approvals
  at HEAD — an auditable trail, not a second identity.
- **`crucible run`** proves the **app** is real: it builds, boots, and drives the real
  artifact against real oracles. A green unit suite and a working app are different claims.
- **`crucible harden`** proves the **tests** are real: diff-scoped mutation testing. A
  mutant that survives on changed code is proof no test checks that behavior, and it names
  the exact test to write.
- **`crucible cover`** is the floor under harden: diff-scoped coverage that names the
  changed functions **no test ever calls**. Not called is not tested, full stop. Coverage
  answers "was it run"; mutation answers "was it checked".

A green suite proves none of these.

## The one law

> A correctness-critical rule is a gate in the required per-change lane, or it is
> explicitly declared advisory with a written rationale. There is no third state.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex"
```

Or from a checkout: `cargo install --locked --path .`

## Five minutes

```sh
crucible init                     # scaffold .crucible/ (adapter, charter, recipes, approvals)
# fill the TODOs: gate runner + highRiskUnits (adapter), build/boot/drive (acceptance), mutation cmd
crucible approve __config__ --by <reviewer>   # a reviewer, not the author, pins the config
crucible approve <gate> --by <reviewer>       # ...and each gate's oracle
crucible doctor                   # is it wired right?
crucible check                    # confirm every gate is honest
```

`init` is idempotent and never overwrites your config. It scaffolds a pre-push hook that
runs `crucible check`. Commit each approval **separately** from the config it blesses
(`check` flags same-commit self-approval at HEAD). Point git at the hook dir:

```sh
git config core.hooksPath .githooks
```

## Dogfood

This repository adopts Crucible on itself. The product under test is the `crucible` binary.

| Arm | What it runs here |
| --- | --- |
| `check` | T1 checkers: own `test-smells` + proof/demo/benchmark suite |
| `run` | build, `--help` boot, doctor/check/test-smells drive oracles |
| `harden` | diff-scoped `cargo mutants` (unit tests only — nested arms deadlock) |
| `cover` | `cargo llvm-cov --bins` on the worktree diff |
| `flake` | `cargo test --bins` twice |

```sh
cargo build -q
./target/debug/crucible doctor   # adoption health
./target/debug/crucible check    # gates honest
./target/debug/crucible run      # the CLI actually runs
./target/debug/crucible harden   # tests bite on the current diff
./target/debug/crucible cover    # changed production functions are executed
./target/debug/crucible flake    # unit suite is deterministic
```

`harden` / `cover` are change-scoped: a clean tree fails closed (nothing to mutate /
empty scope). That is intentional — they prove the *change*, not the whole repo forever.

## Commands

| Command | Proves | Blocks when |
| --- | --- | --- |
| `check` | the gates are real | a declared gate is not wired where it claims, a checker runs unregistered, or an oracle changed without an independent approval |
| `run` | the app is real | the real artifact does not build, boot, or pass its drive oracles |
| `harden` | the tests are real | a mutant survives on changed high-risk code |
| `cover` | the code was run at all | a changed function in a high-risk unit is never called by any test |
| `flake` | the suite is deterministic | a test flips pass/fail across identical runs (a flaky green is a false green) |
| `audit` | (utility) | prints the declared-vs-actual enforcement delta |
| `approve` | (utility) | records an independent oracle approval (`__config__` pins the judge config) |
| `init` | (utility) | scaffolds `.crucible/` config |
| `doctor` | (utility) | adoption health: is `.crucible` wired right? |
| `test-smells` | (utility) | scans files/dirs for reward-hacked tests (skips, focused, assertion-free, tautological) |

## Resource safety

The four arms that spawn a real build or test tree — `run`, `harden`, `cover`, `flake` —
are bounded so a wall of parallel Crucible sessions cannot take the machine down:

- **Machine-wide concurrency gate.** Only N heavy runs execute at once across *every*
  Crucible process on the box (default `1`, capped at core count). Set it persistently
  with `crucible config max-concurrency <N>` — a machine-level file at
  `~/.config/crucible/config.json`, deliberately not repo config, so an agent cannot
  grant itself more slots by editing the repo it is working in. `CRUCIBLE_MAX_CONCURRENCY` overrides it per-shell,
  and `crucible config` / `crucible doctor` show which layer is live. More slots also
  means a lower default memory ceiling per tree (the budget below splits N ways). A
  session that arrives while the slots are full waits (up to `CRUCIBLE_SLOT_WAIT_SECS`,
  default one hour) rather than piling another `cargo` build onto the memory. Slots are OS
  file locks in a fixed machine-wide path (`/tmp/crucible-slots` on unix), so a crashed
  session frees its slot automatically and a per-session `TMPDIR` does not split the
  namespace. `CRUCIBLE_SLOTS_DIR` overrides the path — a deliberate escape hatch for
  per-project gating; setting a different value per session splits the gate, so only do it
  on purpose. If a second `crucible` seems to hang, it is queued.
- **Kernel-enforced containment on Linux.** Where `systemd-run --user --scope` is available,
  each heavy tree runs in a cgroup-v2 scope: `MemoryMax` OOM-kills it the instant it exceeds
  (no poll window), `MemorySwapMax=0` stops swapping around the limit, `TasksMax` caps the
  process count (fork-bomb protection), and tearing down the scope reaches every process in
  its cgroup — including a `setsid` daemon that left the process group. This holds even for
  an orphaned tree: the kernel cap means it cannot tank RAM. Set `CRUCIBLE_NO_CGROUP=1` to
  force the fallback below. (macOS/Windows and non-systemd Linux use the fallback.)
- **Process-tree memory ceiling (fallback + belt).** `run`, `cover`, and `flake` cap their
  tree at a machine-aware default (a fraction of RAM, divided by the concurrency so N
  same-configured trees stay within budget) unless a recipe sets an explicit `memoryMb`;
  `harden` uses a fixed 2048 MiB default, also overridable. The ceiling is measured by
  summing the process group's RSS (a reparented worker still in the group is counted). It is
  *polled*: sustained overuse is killed within the sample interval. Where the cgroup scope is
  active the kernel is the hard backstop; where it is not, a command that allocates several
  GiB between samples can spike — set `memoryMb` low for known-heavy recipes. If total RAM
  cannot be read, the arm runs uncapped and says so.
- **Hard timeout** with whole-process-tree kill (group/JobObject) on every exit path,
  including any work a recipe backgrounds with `&`.
- **Log-capture cap.** The captured stdout/stderr file is polled and the tree killed once
  it crosses ~256 MiB (a fast writer can overshoot by one 20 ms sample, not grow without
  bound). This bounds the *capture file*, not arbitrary files a command writes elsewhere
  (`dd of=big.bin` is not caught) — scope untrusted recipes with a filesystem quota.

A run killed by any cap fails closed — it certifies nothing.

### Containment by platform

| Threat | Linux + `systemd-run` scope | Fallback (macOS, Windows, non-systemd Linux) |
| --- | --- | --- |
| One tree exhausts RAM | Kernel `MemoryMax` OOM-kills it, no poll window | Polled RSS ceiling; a burst between samples can spike |
| Fork bomb | Kernel `TasksMax` | Whole-process-group kill on cleanup |
| `setsid` / double-fork escape | Scope cgroup kill reaches it | **Escapes** the process-group kill |
| Crucible itself `SIGKILL`ed | Orphan survives but is **still RAM-capped** by the cgroup | Orphan survives and is **uncapped** |
| Crucible `SIGTERM`/`SIGINT`/`SIGHUP` | Group + scope torn down | Group torn down (setsid escapes survive) |

On Linux with a user systemd session the first four are contained by the kernel. The
fallback path bounds the realistic case (honest recipes, RAM readable) but leaves the
escapes above — stated rather than hidden.

Remaining on every platform:

- **Windows spawn-before-job window.** A grandchild started in the brief window before the
  Job Object is assigned is not retroactively contained.
- **Diff-discovery git is PATH-resolved.** `harden`/`cover` shell out to `git diff` (from
  PATH) *before* acquiring their slot. That git now runs under a 60-second hard timeout with
  a whole-process-group kill and an output cap, so a hung or flooding `git` can no longer
  hang Crucible or fill the disk outside the gate — but a hostile PATH `git` still runs with
  your privileges. Trust your PATH, or run Crucible where `git` is fixed.

## Portable core, thin adapter

The core is project-agnostic. Each repository adds a small `.crucible/adapter.json` naming
its gate runner, its high-risk (money/checkout) units, and its pre-push hook. One binary
validates any repo whose adapter is filled in. See `docs/ADOPTING.md`.

## For coding agents: the skill

Crucible ships as a Claude Code / Codex **skill** (`skills/crucible/`) so an agent runs
`crucible run` / `harden` / `check` on itself before reporting a task done. The skill is
the inner-loop on-ramp; the CLI runs the same commands in CI and pre-push as the backstop
that does not depend on the agent choosing to check itself. The skill installs from this
repo with no dependency on any other plugin.

## Development

```sh
cargo test                                          # unit + integration
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
