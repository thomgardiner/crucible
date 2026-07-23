---
id: kb-crucible-enforcement-charter
title: The Enforcement Charter
type: rules
description: The Enforcement Charter doctrine, tier model, and the procedure to add, promote, or waive a gate.
---

# The Enforcement Charter

Crucible exists because of one finding that every team running agent-authored
code at scale reports: prose rules do not bind coding agents. Only machine-enforced
gates do. A rule written in a style guide is a suggestion an agent optimizes around.
A rule that fails the build is a constraint it optimizes within.

The Charter turns that finding into a discipline.

## The one law

> A correctness-critical rule is a gate in the required per-change lane, or it is
> explicitly declared advisory with a written rationale. There is no third state.

"Written in the rules doc" is not a state. Every rule is either enforced, or it is
declared unenforced on purpose with a reason. The Gate Ledger records which.

## The four tiers

Every check carries exactly one tier.

| Tier | Where it runs | Timing | Binds? |
| --- | --- | --- | --- |
| T0 | per-edit hook (PostToolUse) | on write | immediate nudge or block |
| T1 | required per-change lane | every change | blocking. The load-bearing tier. |
| T2 | milestone / qualification lane | scheduled | deep, non-blocking on the inner loop |
| T3 | prose only | never | advisory, must carry a reason |

The rule of thumb: any invariant whose violation ships broken product belongs at T1.
The most common failure mode this framework fixes is a strong check that was built
and then parked at T2, where it grades nothing on the change that introduced the bug.
Promoting it from T2 to T1 is usually a wiring change, not new machinery.

## The Gate Ledger

`.crucible/charter.json` is the registry. One row per gate:

```json
{
  "id": "test-trust",
  "rule": "no reward-hacked tests (skips, stubs, copied impls, CI retries)",
  "tier": "T1",
  "checker": "scripts/check-test-trust.mjs",
  "oracleSha256": "<approved digest of the checker>",
  "trustedFiles": [],
  "blockingCondition": "always"
}
```

`crucible check` verifies the ledger is honest and fails the build when any of
these hold:

1. A rule marked as a gate does not resolve to a real checker file.
2. A T1 gate's checker is not actually wired into the required lane (a declared
   gate that grades nothing is a lie).
3. A checker running in the required lane is missing from the ledger (no silent,
   unaccounted gate).
4. An oracle has drifted: the checker's bytes changed away from the pinned
   `oracleSha256` without an independent approval.
5. A prose-only (T3) or advisory rule carries no `reason`.

## Oracles and independent approval

The `oracleSha256` pins the bytes of the judge, separately from the code the judge
grades. This is the anti-tamper property: a change cannot quietly weaken the gate
that would have caught it, because editing the checker breaks its pinned digest and
fails `crucible check`.

Clearing the drift is deliberate and recorded:

```
crucible approve <gate> --by <reviewer> --note "why the checker changed"
```

`approve` re-pins the digest and appends to `.crucible/approvals.json`. The
approval should land as a commit separate from the checker change, so the approver
is not the author. That separation is enforced at the repository's pre-push layer,
which the adapter names; the core provides the record and the drift detection.

## The procedure

**Add a gate.** Write the checker (see `templates/checker.template.mjs`: mask
comments and strings, collect itemized failures, exit non-zero, ship a `.test.mjs`
that proves it fires on a planted violation). Wire it into the required lane. Add a
ledger row at its tier and pin the oracle with `crucible approve`.

**Promote a gate.** Move its invocation from the milestone lane into the required
lane and change its tier from T2 to T1. `crucible check` will now require it to be
wired where it claims to be.

**Waive a gate.** Set the tier to T3 (or `blockingCondition: advisory`) and write a
`reason`. A waiver without a reason fails the check, so an unenforced rule can never
masquerade as an oversight.

## What this framework does not do

It does not judge whether a rule is correct, only whether the repository enforces
what it claims to. It does not run the gates; the repository's gate runner does. It
holds the map honest so a human reading `crucible audit` sees the true enforcement
posture rather than the aspirational one.
