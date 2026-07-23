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
| Arm certified clean | exit 0 **and** success verdict **and** receipt file exists |
| Dry-run / not certified | banner **and** receipt file **absent** |
| Survivors reported | `.crucible/survivors.json` names the mutant |

Positive controls pair with negative ones: if “blocking” is only proven by exit 1,
a broken arm that always exits 1 would fake the proof — so clean paths must also
be proven to exit 0 and mint receipts.

This suite also runs `crucible test-smells` on `tests/` and `src/`. If that fails,
fix the smell — do not weaken the scanner.

```
cargo test --test proof    # just the PROOF tests
cargo test                 # the whole suite, proofs included
```

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

Committing the real cargo-mutants output is deliberate: if a cargo-mutants upgrade
changes the format, this proof fails loudly instead of the gate silently passing
everything.

Live tooling (needs `cargo-mutants`):

```
cd examples/proof/mutation-crate
cargo test            # green: 1 passed
cargo mutants         # real: 1 missed — the green test does not constrain <=
```

## Proof 2: reality arm catches a boot crash

A green unit suite says nothing about whether the real app comes up. The proof
launches a **real subprocess** that either reaches ready or panics on startup:

- healthy → exit 0, “the app actually runs”, **run receipt present**
- crash → exit 1, “crashed on launch”, **no receipt**

## Proof 3: tamper-evidence on the gate

Honest, approved gate (`check` passes + check receipt), then real weakenings:

| Attack | What catches it |
| --- | --- |
| Neuter the checker | fingerprint no longer matches an approval |
| Comment the gate out of the required lane | “not wired in the required lane” |
| Empty `highRiskUnits` | judge-config fingerprint invalid |
| Downgrade T1 → T2, keep approval | tier is in the fingerprint |
| Strip `prePush` / inert pre-push | load-bearing independence wiring |

After each attack is reverted the gate is honest again.

## Proof 4: custom `--recipe` never certifies

Throwaway recipe path may “RUN” the fake app but:

- prints NOT certified
- **writes no receipt**
- **positive control:** same body at `.crucible/acceptance.json` **does** mint a receipt

## Proof 5: cover refuses empty scope

Clean tree vs `HEAD` with a coverage command that would otherwise succeed:

- exit ≠ 0, empty-scope message, **no cover receipt**

## Proof 6: self-scan

`crucible test-smells tests src` on this repository exits 0.

## What is not proven here

These proofs use real captured tool output and real subprocesses, but on minimal
fixtures, not on a large production codebase. They prove the mechanisms work; they
do not prove Crucible has been wired into any particular repository's required lane
(that is adoption, see [ADOPTING.md](ADOPTING.md)).

Independence is **not** cryptographic multi-party approval (single-dev + agents share
one git identity). What is proven: pre-push is load-bearing, same-commit self-approval
at HEAD is flagged, and receipts only mint on canonical certified paths.
