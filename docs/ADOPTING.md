# Adopting Crucible

Adoption is small. The binary is portable; each repo adds only what is specific
to itself under `.crucible/`.

## 1. Scaffold

```sh
crucible init
git config core.hooksPath .githooks
```

`init` writes adapter, charter, recipes, approvals, and a pre-push hook that runs
`crucible check`. It does not overwrite existing files unless you pass `--force`.

## 2. Fill the adapter

`.crucible/adapter.json` points at your machinery:

```json
{
  "repo": "my-app",
  "gateRunner": {
    "command": "make verify",
    "file": "scripts/verify.sh",
    "checkerPattern": "sh (checks/check-[a-z-]+\\.sh)"
  },
  "highRiskUnits": ["payments", "auth"],
  "prePush": ".githooks/pre-push"
}
```

- `gateRunner.file` + `checkerPattern` — how `check` finds checkers in the required lane
  (first capture group = repo-relative checker path).
- `highRiskUnits` — path components where surviving mutants and never-called functions
  block (money, auth, checkout, etc.).
- `prePush` — load-bearing; missing or inert hooks fail `check` / `doctor`.

Also set build/boot/drive in `acceptance.json`, and mutation/coverage/flake recipes
as needed. See `examples/demo/` for a minimal complete project, or
`examples/large-app.*.json` for a fuller shape.

## 3. Seed the ledger and approve

`.crucible/charter.json` lists every gate. One row per checker in the required lane,
plus T3 rows for rules you have not automated yet.

```sh
crucible approve __config__ --by <reviewer>
crucible approve <gate> --by <reviewer>
crucible doctor
crucible check
```

Commit each approval **separately** from the config or checker it blesses.
`check` flags same-commit self-approval at HEAD.

## Day-to-day

```sh
crucible test-smells path/to/tests
crucible check
crucible run
crucible cover
crucible harden
```

`harden` and `cover` measure the **change** (diff vs a base). On a clean tree they
refuse to certify — intentional.

## Agents

Install the skill from this repo (`skills/crucible/`). Agents should run the same
CLI before calling work “tested.” CI and pre-push remain the backstop.
