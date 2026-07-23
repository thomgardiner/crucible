# Examples

## [demo/](demo) — short tour

Minimal project with Crucible wired in:

```sh
cd examples/demo && ./demo.sh
```

Shows `check` catching a weakened gate, `run` catching a boot crash a green unit
test misses, and `harden` catching a test that does not constrain behavior.

## [proof/mutation-crate/](proof/mutation-crate) — live mutation

Reward-hacked Rust crate used by the proof suite. With `cargo-mutants`:

```sh
cd examples/proof/mutation-crate
cargo test      # green
cargo mutants   # one missed mutant — the green test does not constrain <=
```

## Config shapes

`large-app.*.json` are portable adapter / charter / acceptance / mutation examples
for a multi-crate app. Copy, rename, and point commands at your verify/build/mutation
entry points.

## See also

- [docs/PROOFS.md](../docs/PROOFS.md) — `cargo test --test proof`
- [docs/ADOPTING.md](../docs/ADOPTING.md) — adopt in your repo
