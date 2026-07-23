---
id: kb-crucible-adopting
title: Adopting Crucible
type: reference
description: How a repository adopts the Enforcement Charter in three steps — write the adapter, seed the ledger, wire the honesty check.
---

# Adopting Crucible

Adoption is deliberately small. The portable core carries the doctrine, the schema,
and the honesty checker. A repository adds only what is specific to itself.

## 1. Write the adapter

`.crucible/adapter.json` binds the framework to your real machinery. It answers
four questions:

```json
{
  "repo": "battlemage",
  "gateRunner": {
    "command": "grove verify change",
    "file": "scripts/verify.ps1",
    "checkerPattern": "node (scripts/check-[a-z-]+\\.mjs)"
  },
  "changeToUnits": "tools/test-impact.mjs",
  "highRiskUnits": ["target", "cybersource", "amazon", "shopify"],
  "prePush": ".githooks/pre-push"
}
```

- `gateRunner.file` and `gateRunner.checkerPattern` are how `crucible check`
  discovers which checkers actually run in your required lane. The pattern's first
  capture group must be a repo-relative checker path.
- `highRiskUnits` are the money and checkout units where a silent test is
  catastrophic. Gates with `blockingCondition: highRisk` block only when one of
  these is touched.

## 2. Seed the ledger

`.crucible/charter.json` lists every gate. Seed it by walking your gate runner and
recording one row per existing checker, plus a T3 row for every written rule you do
not yet enforce. That first pass is the point: it makes the gap between your rules
doc and your real enforcement visible.

Pin each checker's oracle with `crucible approve <gate> --by <you>`.

## 3. Wire the honesty check into the required lane

Add one T1 step to your gate runner that runs `crucible check`. From that point on
the charter cannot lie: a checker you remove, a gate you misplace, or an oracle you
weaken all fail the required lane.

Register that step in the ledger too. Crucible checks itself.

## Keeping it honest at push time

Point your pre-push hook at two things: a fresh required-lane pass, and a refusal to
accept a change to the charter or any pinned oracle without a matching approval
recorded by someone other than the author. The core detects the drift; the pre-push
policy enforces the separation.
