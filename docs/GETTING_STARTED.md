# Getting started

From zero to a healthy `crucible doctor` in a few minutes.

## 1. Install the CLI

**Release installer (recommended):**

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy ByPass -c \
  "irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex"
```

**From a git checkout:**

```sh
cargo install --locked --path .
```

Confirm:

```sh
crucible --version
which crucible   # must be on PATH for pre-push and agents
```

## 2. Adopt a repository

```sh
cd /path/to/your/repo
crucible init
crucible doctor
```

`init` writes:

| Path | Role |
| --- | --- |
| `.crucible/*` | Adapter, charter, recipes, approvals |
| `scripts/verify.sh` | Required-lane gate runner |
| `checks/check-smoke.sh` | Placeholder T1 checker (wiring only — replace with a real gate) |
| `.githooks/pre-push` | Runs `crucible check` on push |

If `core.hooksPath` was unset, init sets it to `.githooks` for this repo.

## 3. Pin the starter config

Approving a gate rewrites the charter, so pin **gates first**, then `__config__`:

```sh
crucible approve smoke --by "$USER"
crucible approve __config__ --by "$USER"
# Commit approvals in a separate commit from the config files.
crucible doctor
crucible check
```

You should see an honest charter and a clean check.

## 4. Point recipes at your real tools

Edit only what you will use:

| File | Arm |
| --- | --- |
| `.crucible/acceptance.json` | `crucible run` |
| `.crucible/mutation.json` | `crucible harden` |
| `.crucible/coverage.json` | `crucible cover` |
| `.crucible/flake.json` | `crucible flake` |
| `.crucible/adapter.json` → `highRiskUnits` | where survivors/never-called block |

Replace `checks/check-smoke.sh` and the `smoke` charter row with real gates when ready.

After recipe or gate edits (gates first, then config):

```sh
crucible approve <gate-id> --by "$USER"
crucible approve __config__ --by "$USER"
```

## 5. Day-to-day

```sh
crucible test-smells path/to/tests
crucible check
crucible run          # once acceptance.json is filled
crucible cover        # once coverage.json is filled; needs a dirty diff
crucible harden       # once mutation.json is filled; needs a dirty diff
```

## Agents (optional)

Install the skill/plugin from this repo so Claude Code / Codex load `skills/crucible/`.
The skill is guidance; the CLI is the engine. See `after-install.md` after installing
the plugin.

## More

- [ADOPTING.md](ADOPTING.md) — adapter and charter details  
- [PROOFS.md](PROOFS.md) — what the product proves in CI  
- [RESOURCES.md](RESOURCES.md) — concurrency and memory bounds  
