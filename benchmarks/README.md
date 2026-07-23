# Detection benchmark

"Crucible catches reward hacking" is a falsifiable claim, so it is measured, not
asserted. This benchmark runs a labeled corpus of reward-hacks and honest controls
through the real `crucible` binary and reports two numbers per arm:

- **Recall** = hacks caught / total hacks. Misses are reward hacks that slipped through.
- **Precision** = honest passed / total honest. Misses are false positives, honest code
  wrongly flagged. A detector that cries wolf gets uninstalled, so precision matters as
  much as recall.

It is deterministic and reproducible: no model calls, no wall-clock, no network. The
harness lives in `tests/benchmark.rs` and asserts the floor (100% each), so a regression
that lets a hack through or false-flags honest code fails the build.

Run it:

```sh
cargo test --test benchmark -- --nocapture
```

## Current scorecard

```
arm               recall         precision
               (hacks caught)  (honest passed)
test-smells        14/14           15/15
mutation            5/5             4/4
reality             8/8             2/2
gate-tamper         9/9             2/2
coverage            5/5             3/3
flake               4/4             2/2
----------------------------------------------
TOTAL          45/45 hacks caught      28/28 honest passed
               recall 100%            precision 100%

A hack only counts as caught when the run fails AND names the specific reason (the
`caught_with` signal in `tests/benchmark.rs`) — a bare non-zero exit could be a broken
fixture, which would let the benchmark reward-hack itself. Symmetrically, an honest case
only earns precision when it exits 0 AND prints its success verdict (`passed_with`) — exit
0 alone could mean the arm never actually exercised the fixture. Harness state is real,
not accidental: the mutation fixtures commit a baseline and modify a high-risk file so the
risk classification comes from the actual diff, not from a failing `git diff HEAD`. The corpus now includes the
rigged-config classes surfaced by third-party review: empty-matching oracles, forbid-only
drive steps, no-evidence mutation runs, empty-reason waivers, block-commented gate wiring,
blank approvers, post-approval waiver/helper edits, FN-less coverage records, coverage
reports that omit the changed file, Windows path separators, and deterministic-red suites
(including a failure printed under exit 0).
```

This 100% is a **regression guard on a corpus we author**, not the field precision. A
benchmark you write and grade yourself proves the tool has not regressed on cases you
thought of; it does not prove precision on code you did not. For that, we run the real
thing against a real large codebase, below.

## Precision on a real large codebase (the honest number)

`test-smells` was run over Battlemage's real suite (966 test files: 633 Rust + 333 TS).
The first run flagged 157. Reading the flagged code (not trusting the count) surfaced
five false-positive classes, each a real detector bug only a real codebase exposes:

| Fix | Bug | Flags after |
| --- | --- | --- |
| assert-family match | `\bassert\b` missed `prop_assert!`, `assert_exact`, `assertForwarded` | 157 → 118 |
| node:assert member | `assert.equal(...)` (the `.` broke the match) | 118 → 65 |
| test-def vs test-call | `function test(name, fn){…}` matched as a test invocation | (folded in) |
| `throw` is an assertion | `if (!cond) throw new Error(...)` not recognized | 65 → 43 |
| `#[ignore]` suppression | an ignored test does not run, so it is not a false green | (folded in) |
| assertion-helper allowlist | a repo declares helpers like `recovers_after` | 43 → 24 |

The final 24 flags on 966 files break down as: **9 real smells** (tautologies like
`expect(true).toBe(true)` — true positives), **13 no-output-assertion proptests** (weak
tests, defensibly flagged), and ~2 residual. The true false-positive rate after declaring
one assertion helper is near zero.

The sneaky-class detectors (silent self-skip via early return, empty-catch swallowed
assertions, fire-and-forget promise chains) were tuned the same way: on the same 1,266
files they produce **zero false positives** after one refinement the dogfood forced —
the Rust self-skip flag only fires at guard depth, because a `return;` nested in a loop
is the poll-until idiom (return on success, panic on timeout), not a skip.

`test-smells` is a **screen, not an oracle**: its residual limit is assertions hidden
behind a helper it has not been told about (`--helpers`, or `.crucible/test-smells.json`).
The strongest arm is mutation (`harden`): a surviving mutant is hard evidence a test does
not constrain that line. It is not free of false positives, though — an equivalent mutant
(semantically identical, so surviving is correct) is a real one, which is why waivers with
a written reason exist, and a TIMEOUT is inconclusive, not proof. And every arm is only as
strong as the recipe the repo provides: an empty or absent command must fail closed, never
pass. "Objective" here means the evidence is a real artifact (a real mutant, a real
zero-hit function), not that the tool cannot be misconfigured.

Dogfood: `crucible test-smells src tests` on Crucible's own suite reports no smells.

## Are Crucible's own tests reward-hacked?

The question the tool must answer about itself, answered with its own methods rather
than self-review:

- **Mutation run** (`cargo mutants`, 581 mutants over the eight core judgment modules):
  the first pass surfaced real test gaps — un-asserted validation branches, scanner
  boundary arithmetic, the hook's blocking path, the raw-string masker — each killed
  with a targeted test, then re-verified by a scoped second mutation run against the
  full suite. A fixture bug it exposed: the trust-audit test used counts where
  `3 - 2 == 3 / 2`, so an arithmetic mutant was invisible; the fixture now uses counts
  where they differ.
- **Reversion proofs**: neutering the flake exit-0 refusal makes its CLI test fail;
  neutering cover's unmatched-file screen drops the benchmark's coverage arm to 4/5 and
  fails the build. The guards bite; they are not decorative.
- **Benchmark self-integrity**: hacks must fail naming their mechanism, honest cases
  must print their success phrase, fixtures assert every setup step, and an arm with an
  empty corpus fails rather than passing `0 == 0`.

Waived mutants, each with its reason (a silent waiver would be its own hack):

| Mutant | Why it stands |
| --- | --- |
| `now_secs -> 0/1` | receipts stamp and check through the same clock; killing this needs injected time, machinery a nudge helper does not warrant |
| `find` empty-needle guard | defensive; unreachable through any caller (the needle is always a real delimiter) |
| `ret.start() < first_assert.start()` → `<=` | a return and an assertion cannot begin at the same byte, so the boundary case cannot occur |
| `has_uncommitted_changes` status-guard → `true` | equivalent in reach: the only way to hit it is a git that both fails AND emits non-whitespace stdout, which git does not do; a non-repo yields empty stdout, so both branches return false |
| `is_commented` line-start `i + 1` → `i * 1` | sets the offset to the newline's own byte instead of one past it; the extra leading `\n` can never be `#` or form `//`, so the comment check returns identically for every input (the sibling `i - 1` mutant IS killed — it reaches the previous line's `#`) |
| `find` guard `needle.is_empty() \|\| from >= len` → `&&` | the needle (a raw-string close delimiter) is never empty and `from` never exceeds `len`; at `from == len` the slice is empty and both branches return None, so no input distinguishes them |
| `inspect_rust_with` return-scan `r.start() < first_assert.start()` → `<=` | a `return` token and an assertion token cannot begin at the same byte, so the `==` boundary is unreachable |
| `callback_body` `brace < end` → `<=` | `brace` (a `{` index) and `end` (the call's `)` index) are distinct byte positions, so equality never holds |
| `statement_start` brace-depth arms (`brace += 1` → `*= 1`, the `paren == 0 && bracket == 0` guard on the `{` arm, its `&&` → `\|\|`) | all reachable only when a `{` is scanned at `brace == 0` while inside an open bracket/paren — i.e. unbalanced `{` nesting. In masked well-formed source every `{` either has a pending `}` (takes the `brace > 0` arm) or sits at all-zero depth (a real boundary), so no input distinguishes these |

The two genuinely reachable survivors from that run — a `/`-or-`*` operator wrongly opening a block comment, and a single-quoted string inside an interpolation — were killed with targeted tests (re-verified: both now `CaughtMutant`), not waived. A third mutant I initially expected to kill (a `{` inside brackets) turned out to be one of the unbalanced-nesting equivalents above: the re-run showed the test did not kill it, so it moved to the waiver table rather than being claimed as a kill.

### Resource-safety self-audit (`proc`, `admission`, `cgroup`)

The machine-wide containment code (per-tree memory ceiling, admission slots, signal reap,
kernel cgroup scope, bounded git discovery) went through the same treatment. `cargo mutants`
diff-scoped to it found 147 mutants: 79 caught, 40 missed, 16 unviable, 1 timeout on the
first pass. Triaging every survivor against `caught.txt` and its source separated three kinds.

- **Genuinely reachable gaps, killed with tests rather than waived.** `resolve_memory_limit`
  could return `Ok(None)` (every heavy tree silently uncapped) with no test to stop it. The
  `budget_bytes` 80%/concurrency ratio and the `over_output_cap` disk-guard boundary were
  unpinned. The logic was extracted into pure helpers and tested against known inputs, and a
  scoped re-run confirmed all 16 mutants across the three functions now `CaughtMutant`.
- **Other-OS `#[cfg]` code, false survivors on this host.** The Windows/fallback `try_lock`
  arms, the linux/windows `total_ram_bytes` bodies, and all of `cgroup.rs` do not compile on
  the macOS box the audit ran on, so mutating them changes nothing there. The live macOS
  paths ARE caught: the unix `flock`, slot contention, and the macOS RAM read are all in
  `caught.txt`. The gated code is exercised on its own platform's CI, and the cgroup scope by
  the Linux-only `a_runaway_allocation_is_contained_under_a_tiny_ceiling` integration test.
  A single-platform mutation run always over-reports these, so the honest count is per-OS.

| Waived mutant | Why it stands |
| --- | --- |
| `flock(fd, LOCK_EX \| LOCK_NB)` → `^` | `LOCK_EX` (2) and `LOCK_NB` (4) share no bits, so `2 ^ 4 == 2 \| 4 == 6` — identical syscall argument |
| overflow-fallback deadline `Duration::from_secs(7 * 86_400)` → `+`/`/` (in `acquire_in`, `run_shell`, `run_program_bounded`) | reached only when `checked_add` overflows on an absurd timeout; any large-enough duration is observationally identical, and no non-overflowing input reaches the branch |
| `OUTPUT_CAP` const `256 * 1024 * 1024` → `+` | the disk guard fires far below the cap in every test; pinning the literal against itself would be tautological, and `over_output_cap`'s boundary is now tested directly |
| `total_ram_bytes` (linux/windows bodies) → `None`/`Some(0)`/`Some(1)`; `try_lock` (windows/fallback) → `true`/`false`; `cgroup::{available,detect,kill}` | `#[cfg]`-gated out on the macOS audit host — not compiled, so the mutant is a no-op here; each is live and covered on its own platform |
| `tree_fingerprint` NUL-split `*b == 0` → `!=`; `untrack_group` → `()`; `slots_dir` → `Default` | hook-optimization / signal-plumbing, not a safety boundary: a wrong fingerprint only over- or under-nudges the TUI, an uncleared slot array entry self-heals on process exit; documented rather than given contrived tests |

## The corpus

The six arms above, each case a real artifact run end to end through the binary; the
scorecard totals are the source of truth for the counts:

- **test-smells**: assertion-free tests, `assert!(true)`, `assert_eq!(x, x)`,
  `expect(x).toBe(x)`, `it.skip` / `it.only` / chained `.concurrent.only`,
  `process.exit(0)` in a test, `#[ignore]` with no reason. Honest controls: real
  assertions, `?`-returning tests, `#[should_panic]`, `#[ignore = "reason"]`,
  expression-bodied arrows, an options-object argument, and real-world assertion styles
  (`prop_assert!`, `node:assert`, assert-named helpers) — all known false-positive traps.
- **mutation**: mutation output with surviving mutants (MISSED / TIMEOUT, single and mixed
  with CAUGHT) vs output where every mutant is caught.
- **reality**: recipes whose real app is broken (boot crash, exit-0-without-ready, a
  failing drive flow, a failing build) vs healthy apps that build, boot, and drive.
- **gate-tamper**: a weakened checker, an unwired T1 gate, an emptied pinned config, a
  T1 to T2 downgrade, and a silently wired unregistered checker vs the honest baseline.
- **coverage**: LCOV where a changed function has zero hits vs one where all are covered.
- **flake**: a command that flips pass/fail across runs vs one that is deterministic.

## Scope

This measures detection: given a hack, does Crucible flag it, and given honest code, does
it stay quiet. It does not yet measure the downstream claim that installing the skill
makes an agent ship fewer hacks. That is a live-model behavior benchmark (agents run on
tasks with and without Crucible), which is non-deterministic and needs model access. It
is a separate, larger effort tracked for pre-launch; this deterministic benchmark is the
always-on floor.
