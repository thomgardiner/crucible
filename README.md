# crucible

Honesty layer for the tests you already run. Crucible does not replace
`cargo test`, Vitest, or Playwright. It answers a narrower question: **did this
change get verified in ways a green unit suite alone can miss?** That matters
when most of the code (and many of the tests) come from coding agents.

## Install

macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.sh | sh
```

Windows PowerShell:

```powershell
$ErrorActionPreference = "Stop"
irm https://github.com/thomgardiner/crucible/releases/latest/download/crucible-installer.ps1 | iex
```

The installer verifies the release checksum and also installs `crucible-update`.
Source install: `cargo install --git https://github.com/thomgardiner/crucible --locked`.
Optional Claude Code / Codex skill: see [after-install.md](after-install.md).
Full walkthrough: [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md).

## Use

```sh
cd your-project
crucible init                              # scaffold + smoke gate + pre-push
crucible approve smoke --by "$USER"        # gates first (rewrites charter)
crucible approve __config__ --by "$USER"   # separate commit from config
crucible doctor && crucible check
```

Day to day:

```sh
crucible test-smells path/to/tests
crucible check
crucible run          # needs acceptance.json filled
crucible cover        # needs coverage recipe + a dirty diff
crucible harden       # needs mutation recipe + a dirty diff
```

| Command | Question |
| --- | --- |
| `crucible check` | Are the project’s gates still honest — wired, approved, not quietly weakened? |
| `crucible run` | Does the real app build, boot, and pass its drive oracles? |
| `crucible harden` | Do the tests constrain the changed code? (diff-scoped mutation) |
| `crucible cover` | Were the changed functions executed at all? |
| `crucible flake` | Is the suite deterministic across identical runs? |
| `crucible test-smells` | Are there hollow tests (no assert, tautologies, silent skips)? |

Use the arms that match the risk of the change. `harden` and `cover` are
change-scoped: on a clean tree they refuse to certify (nothing to measure).

Exit codes: 0 success, 1 domain refusal (gate red, hollow verification), 2
usage or infrastructure error.

## How it works

- Sits on top of your existing suite; fails closed when verification is hollow.
- Catches green suites that never assert, apps that panic on boot, never-called
  high-risk code, unwired or weakened gates, and “verified” claims that only
  checked wiring.
- Contrast proofs live under `cargo test --test proof` — table and how to
  reproduce them: [docs/PROOFS.md](docs/PROOFS.md).
- A Claude Code / Codex skill under `skills/crucible/` steers agents to the same
  CLI; CI and pre-push remain the backstop.

More: [docs/](docs/README.md) (adopting, resources, methodology).

## License

MIT
