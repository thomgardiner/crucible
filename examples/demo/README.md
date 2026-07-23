# Crucible demo

A complete, tiny project with Crucible wired in. It needs only `sh` and the `crucible`
binary: no node, no mutation toolchain. One command shows all three arms catching a real
problem and then passing honest code.

```
cd examples/demo
./demo.sh
```

## What you are looking at

```
demo/
  app/           a real (tiny) app: a boot script plus a test with real assertions
  checks/        a real gate (no eval() in app code)
  scripts/       the "required lane" runner that invokes the gate
  .crucible/     the config: adapter, charter (the gate ledger), recipes, approvals
```

`.crucible/` is everything a repo adds to adopt Crucible. `approvals.json` was produced
by running `crucible approve`, which is why `crucible check` passes out of the box.

## The three arms, and what each proves

**`crucible check`: the gates are real.** It confirms every gate the charter declares is
actually invoked in the required lane, and that no gate's checker or config changed
without an independent approval. The demo neuters the checker to show it caught.

**`crucible run`: the app is real.** It builds, boots, and drives the actual app. The demo
injects a boot crash while the unit tests stay green, and `crucible run` reports BROKEN
with the panic.

**`crucible harden`: the tests are real.** It runs diff-scoped mutation testing; a
surviving mutant is proof no test checks that behavior, and it names the exact test to
write. The demo replays a real captured cargo-mutants survivor so it needs no toolchain.
For a live `cargo mutants` run, see [../proof/mutation-crate](../proof/mutation-crate).

## Try breaking it yourself

- Add `eval("1")` to `app/app.sh`, then `crucible check --repo .` reports the gate found it.
- Delete the `check-no-eval.sh` line from `scripts/verify.sh` and run check: it reports the
  gate is declared but no longer wired.
- Set `CRUCIBLE_DEMO_BUG=1` and run `crucible run --repo .`: BROKEN, even though the tests pass.

## Adopting this in your repo

Copy the `.crucible/` shape, point the recipes at your real gate runner and mutation
command, list your money/checkout paths in `highRiskUnits`, pin the config, and run
`crucible approve` once per gate. See [../../docs/ADOPTING.md](../../docs/ADOPTING.md).
