---
id: kb-crucible-proofs
type: reference
title: What Crucible Proves, and How to Reproduce It
description: The empirical, deterministic proofs that Crucible catches faked tests, faked gates, and dry-run certification cheats — and the meta-rule that those proofs must bite.
---

# What Crucible Proves

A testing framework that asks you to trust its claims has the same problem as the
green suite it is meant to police. So the claims here are not prose. Each one is a
committed, deterministic proof in `tests/proof.rs` that reproduces on every
`cargo test`, plus a live demo you can run against real tooling.

## Meta-rule (do not clown yourself)

**A proof that only matches a success banner without a material side-effect is a
reward-hack of this library.** Every certification claim asserts:

| Claim | Material check (required) |
| --- | --- |
| Arm blocked a hack | exit ≠ 0 **and** named signal in stdout/stderr |
| Arm certified clean | exit 0 **and** success verdict **and** receipt file exists **and** magic header |
| Dry-run / not certified | banner **and** receipt file **absent** |
| Survivors reported | `.crucible/survivors.json` names the mutant |
| Gate attack caught | exit ≠ 0 **and** named signal (message alone is warn-theater) |

Positive controls pair with negative ones: if “blocking” is only proven by exit 1,
a broken arm that always exits 1 would fake the proof — so clean paths must also
be proven to exit 0 and mint receipts.

This suite also runs `crucible test-smells` on `tests/` and `src/`. If that fails,
fix the smell — do not weaken the scanner.

```
cargo test --test proof    # just the PROOF tests
cargo test                 # the whole suite, proofs included
```

## Contrast table (the body of proof)

Things that look “done” without Crucible, and what blocks them with it:

| # | Without Crucible (green / silent) | With Crucible | Proof |
| --- | --- | --- | --- |
| 1 | Hollow suite: `assert!(true)`, no real assertion — `cargo test` passes | `test-smells` fails closed | PROOF 7 |
| 2 | Green unit suite, function body replaceable with `true` | `harden` names survivor + survivors.json, no receipt | PROOF 1, 12 |
| 3 | Green unit suite, app panics on boot | `run` → BROKEN, no receipt | PROOF 2 |
| 4 | Never-called high-risk function; other symbols covered | `cover` blocks, no receipt | PROOF 8 |
| 5 | Agent only runs `check`, claims verified | Stop hook still **blocks** | PROOF 9 |
| 6 | `echo` timestamp into a receipt file | Stop still **blocks** (no magic) | PROOF 10 |
| 7 | Mutation cmd finds 0 mutants (“clean”) | `harden` refuses to certify | PROOF 11 |
| 8 | Leftover green LCOV on disk | `cover` refuses stale report | PROOF 13 |
| 9 | Custom throwaway recipe “RUNS” | Dry-run: NOT certified, no receipt | PROOF 4 |
| 10 | Empty cover scope vs HEAD | Refuse empty scope | PROOF 5 |
| 11 | Weaken checker / unwired gate / inert pre-push | `check` fails closed | PROOF 3 |
| 12 | Crucible’s own suite is hollow | Self `test-smells` fails the build | PROOF 6 |

## Proof 1: mutation catches a reward-hacked test

A test can pass `cargo test` while asserting nothing about the behavior it claims
to cover. `examples/proof/mutation-crate` is exactly that: `should_buy` uses `<=`,
and its one test only checks a case where the answer is `true`, so replacing the
whole function with `true` still passes.

The deterministic proof runs `crucible harden` against the **real bytes**
cargo-mutants emitted on that crate, committed in
`tests/fixtures/cargo-mutants-real.txt`:

- exit 1, named survivor, survivors.json populated, **no** harden receipt
- **positive control:** clean mutants output → exit 0 **and** harden receipt

Live tooling (needs `cargo-mutants`):

```
cd examples/proof/mutation-crate
cargo test            # green: 1 passed
cargo mutants         # real: 1 missed — the green test does not constrain <=
```

## Proof 2: reality arm catches a boot crash

- healthy → exit 0, “the app actually runs”, **run receipt present**
- crash → exit 1, “crashed on launch”, **no receipt**

## Proof 3: tamper-evidence on the gate

| Attack | What catches it |
| --- | --- |
| Neuter the checker | fingerprint no longer matches an approval |
| Comment the gate out of the required lane | “not wired in the required lane” |
| Empty `highRiskUnits` | judge-config fingerprint invalid |
| Downgrade T1 → T2, keep approval | tier is in the fingerprint |
| Strip `prePush` / inert pre-push (`exit 0`, `\|\| true`) | load-bearing independence wiring |

## Proof 4: custom `--recipe` never certifies

- prints NOT certified, **writes no receipt**
- **positive control:** same body at `.crucible/acceptance.json` **does** mint a receipt

## Proof 5: cover refuses empty scope

- exit ≠ 0, empty-scope message, **no cover receipt**

## Proof 6: self-scan

`crucible test-smells tests src` on this repository exits 0.

## Proof 7: hollow tests green under cargo, fail under test-smells

Temp crate with assertion-free + `assert!(true)` tests:

- `cargo test` → exit 0 (control: “normal green”)
- `crucible test-smells` → exit ≠ 0, names the hollow pattern

## Proof 8: never-called high-risk function

LCOV shows `charge` with FNDA:0 and `refund` hit. Cover on a dirty `pay.rs`:

- exit ≠ 0, names `charge`, **no** cover receipt

## Proof 9: check-only does not clear Stop

Adopted dirty repo, real `crucible check` mints a check receipt, then `crucible hook stop`:

- decision **block** (check is not a verifying arm)

## Proof 10: forged receipt does not clear Stop

Dirty adopted repo, receipt file with only a timestamp (no magic header):

- Stop still **blocks**

## Proof 11: zero mutants refuse certify

`Found 0 mutants to test` → harden exit ≠ 0, **no** receipt

## Proof 12: live contrast (mutation-crate)

Same as PROOF 1’s story, re-stated as an explicit control:

- `cargo test` in `examples/proof/mutation-crate` → green
- harden on the real MISSED capture → block + survivors.json

## Proof 13: stale LCOV cannot certify

Coverage cmd leaves an old `lcov.info` on disk; cover run starts later:

- refuse stale report, **no** receipt

## What is not proven here

These proofs use real captured tool output and real subprocesses, but on minimal
fixtures, not on a large production codebase. They prove the mechanisms work; they
do not prove Crucible has been wired into any particular repository's required lane
(that is adoption, see [ADOPTING.md](ADOPTING.md)).

Independence is **not** cryptographic multi-party approval (single-dev + agents share
one git identity). What is proven: pre-push is load-bearing, same-commit self-approval
at HEAD is flagged, and receipts only mint on canonical certified paths.
