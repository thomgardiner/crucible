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

`crucible init` scaffolds `.githooks/pre-push` that runs `crucible check`. Point
`git config core.hooksPath .githooks` (or install the hook into `.git/hooks`) so it
actually fires. **`check` and `doctor` fail if `adapter.prePush` is missing, the file
is gone, or the hook does not run `crucible check`** — so independence is a verified
wiring fact, not a claim on a dead adapter field.

Also commit each `crucible approve` **separately** from the config/checker it blesses.
`check` flags when the approvals log was last committed together with judge config
(same-commit self-approval). That is the strongest honest trail under a single-developer
+ agents model (agents share the developer's git identity; cryptographic multi-party
approval is out of scope — see POSITIONING.md).
