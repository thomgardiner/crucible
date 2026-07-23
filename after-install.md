# Crucible plugin installed

You have the skill, slash commands, and Stop-hook nudge.

## Install the CLI (required)

The plugin is not the engine. Put `crucible` on PATH:

```sh
# From a release (recommended)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy ByPass -c \
  "irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex"
```

```sh
# From a source checkout
cargo install --locked --path /path/to/crucible
```

```sh
crucible --version
```

## Adopt a repo

```sh
cd /path/to/your/project
crucible init
crucible approve smoke --by "$USER"
crucible approve __config__ --by "$USER"
crucible doctor && crucible check
```

Hooks only run in directories that contain `.crucible/`.

## Slash commands

| Command | Action |
| --- | --- |
| `/crucible` | Full verify path (skill POLICY) |
| `/crucible-init` | `crucible init` |
| `/crucible-doctor` | `crucible doctor` |
| `/crucible-check` | `crucible check` |
| `/crucible-run` | `crucible run` |
| `/crucible-harden` | `crucible harden` |
| `/crucible-config` | concurrency settings |

## Stop nudge

In an adopted repo, finishing with dirty work and no recent `run`/`harden` prompts
you to verify first. Silence with `CRUCIBLE_NO_NUDGE=1` if needed.

Full procedure: skill `POLICY.md`. Product docs: repo `docs/GETTING_STARTED.md`.
