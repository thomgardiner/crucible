---
id: kb-crucible-proofs
title: What Crucible Proves, and How to Reproduce It
type: reference
description: The empirical, deterministic proofs that Crucible catches a faked test and a faked gate, and the exact commands to reproduce them.
---

# What Crucible Proves

A testing framework that asks you to trust its claims has the same problem as the
green suite it is meant to police. So the claims here are not prose. Each one is a
committed, deterministic proof in `tests/proof.rs` that reproduces on every
`cargo test`, plus a live demo you can run against real tooling.

Run the deterministic proofs:

```
cargo test --test proof    # just the PROOF tests
cargo test                 # the whole suite, proofs included
```

## Proof 1: the mutation gate catches a reward-hacked test

A test can pass `cargo test` while asserting nothing about the behavior it claims
to cover. `examples/proof/mutation-crate` is exactly that: `should_buy` uses `<=`,
and its one test only checks a case where the answer is `true`, so replacing the
whole function with `true` still passes.

The deterministic proof runs `crucible harden` against the **real bytes**
cargo-mutants emitted on that crate, committed in
`tests/fixtures/cargo-mutants-real.txt`:

```
Found 3 mutants to test
MISSED   src/lib.rs:4:5: replace should_buy -> bool with true in 0s build + 0s test
3 mutants tested in 1s: 1 missed, 2 caught
```

The proof asserts the gate parses that MISSED line, blocks, and emits the exact
next test to write. Committing the real output is deliberate: if a cargo-mutants
upgrade changes the format, this proof fails loudly instead of the gate silently
passing everything.

To see it end to end on live tooling (needs `cargo-mutants` installed):

```
cd examples/proof/mutation-crate
cargo test            # green: 1 passed
cargo mutants         # real: 1 missed — the green test does not constrain <=
```

Add a boundary assertion (`assert!(!should_buy(11, 10))`) and `cargo mutants`
reports every mutant caught. Both versions pass `cargo test`; only mutation tells
them apart.

## Proof 2: the reality arm catches a boot crash

A green unit suite says nothing about whether the real app comes up. The proof
launches a **real subprocess** that either reaches ready or panics on startup (a
stand-in for a failed DB migration), with the same passing unit test either way:

- healthy process, oracle `app: ready` matches: verdict `RUNS`.
- process panics on boot: verdict `BROKEN`, with the panic captured in the boot
  output.

This is what `crucible run` adds over the unit suite: the real binary either comes
up or it does not, which cannot be faked green the way a test count can.

## Proof 3: the tamper-evidence catches a faked gate

The subtle reward hack is not faking a test; it is weakening the gate that would
catch you. The proof establishes an honest, approved gate (`crucible check` passes),
then applies four real weakenings and asserts each is caught:

| Attack an agent would try | What catches it |
| --- | --- |
| Neuter the checker so it never fires | the checker's pinned fingerprint no longer matches an approval |
| Comment the gate out of the required lane | "declared T1 but not wired in the required lane" |
| Empty `highRiskUnits` so nothing blocks | the pinned judge-config fingerprint no longer matches an approval |
| Downgrade the gate T1 to T2, keep its approval | tier is in the fingerprint, so the approval is invalidated |

After each attack is reverted the gate is honest again, so it fails precisely when
enforcement is weakened, not on everything. The last two attacks are the exact
false-green paths an adversarial review found and that were then fixed, so this
proof also guards those fixes against regression.

## What is not proven here

These proofs use real captured tool output and real subprocesses, but on minimal
fixtures, not on a large production codebase. They prove the mechanisms work; they
do not prove Crucible has been wired into any particular repository's required lane
(that is adoption, see [ADOPTING.md](ADOPTING.md)). And the approval model's trust
root (that the approver is not the change's author) is enforced at the pre-push
layer, which these in-process proofs do not exercise.
