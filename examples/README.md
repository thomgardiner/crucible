# Examples

Runnable proof that Crucible catches what it claims to. Start with the demo.

## [demo/](demo): the 30-second tour

A complete tiny project with Crucible wired in. Runs with just Node.

```
cd examples/demo && ./demo.sh
```

Watch all three arms in one run: `crucible check` catches a weakened gate,
`crucible run` catches a boot crash the green test misses, and `crucible harden`
catches a test that does not constrain behavior.

## [proof/mutation-crate/](proof/mutation-crate): the live mutation run

The real reward-hacked Rust crate behind the deterministic proof. With
`cargo-mutants` installed:

```
cd examples/proof/mutation-crate
cargo test      # green: 1 passed
cargo mutants   # real: 1 missed — the green test does not constrain <=
```

Add a boundary assertion and `cargo mutants` reports every mutant caught. Both pass
`cargo test`; only mutation tells them apart.

## Config references

`battlemage.*.json` are real-shaped adapter, charter, acceptance, and mutation recipes
for a large Rust + Tauri app, showing how a production repo wires in.

## See also

- [docs/PROOFS.md](../docs/PROOFS.md): the deterministic proofs (`cargo test --test proof`)
- [docs/ADOPTING.md](../docs/ADOPTING.md): turning it on in your repo
