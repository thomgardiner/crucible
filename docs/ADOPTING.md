# Adopting Crucible

Adoption is small. The binary is portable; each repo adds only what is specific
to itself under `.crucible/`.

## 1. Scaffold

```sh
crucible init
```

`init` writes adapter, charter, recipes, approvals, a minimal smoke gate
(`scripts/verify.sh` + `checks/check-smoke.sh`), and a pre-push hook. If
`core.hooksPath` is unset, it sets `.githooks` for this repo. Existing files are
left alone unless you pass `--force`.

Prefer the short path in [GETTING_STARTED.md](GETTING_STARTED.md).

## 2. Adapter fields that matter

`.crucible/adapter.json` (filled with working defaults by `init`):

- `gateRunner.file` + `checkerPattern` — how `check` finds checkers (first capture
  group = repo-relative path).
- `highRiskUnits` — path stems where survivors / never-called functions block.
- `prePush` — load-bearing; missing or inert hooks fail `check` / `doctor`.

Fill `acceptance.json` (and mutation/coverage/flake) for the arms you use.
See `examples/demo/` or `examples/large-app.*.json`.

## 3. Approve

```sh
# Gates first (gate approve rewrites the charter), then judge config.
crucible approve smoke --by <reviewer>    # or your real gate ids
crucible approve __config__ --by <reviewer>
crucible doctor && crucible check
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
