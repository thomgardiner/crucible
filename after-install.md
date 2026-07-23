# Crucible installed

Crucible has two parts. The plugin you just installed ships the skill, the slash
commands, and the Stop-hook nudge. The engine is the `crucible` CLI, installed
separately:

```sh
cargo install --locked --path .        # from a checkout
# or the shell / powershell installer from the releases page
```

Once `crucible` is on PATH, the hooks activate automatically in any repo that has a
`.crucible/` directory. Nothing fires in repos that have not adopted Crucible.

## Commands

- `/crucible`          verify this repo: run, harden, check
- `/crucible-run`      does the app actually build, boot, and drive?
- `/crucible-harden`   do the tests bite? (diff-scoped mutation)
- `/crucible-check`    are the gates honest?
- `/crucible-doctor`   is `.crucible` wired right?
- `/crucible-init`     adopt Crucible in this repo

## The skill

The bundled skill is available as `crucible`. It tells the agent to verify with
`crucible run` / `harden` / `check` before reporting a change done.

## The nudge

In a repo with a `.crucible/` directory, when you finish with uncommitted changes and no
recent verification, a Stop hook reminds you to run `crucible run` / `harden` first. Set
`CRUCIBLE_NO_NUDGE=1` to silence it.
