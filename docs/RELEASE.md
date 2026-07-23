# First public cut checklist (v0.1.0)

Not a marketing doc — the list that has to be true before tags and installers.

## Product bar (why this exists)

Agents lie that tests passed. Ship only if the default path makes that expensive:

- [x] Hollow tests: `test-smells`
- [x] Gate weakening: `check` + approvals + load-bearing pre-push
- [x] Green suite, dead app: `run` + receipts
- [x] Tests that do not constrain behavior: `harden` + survivors
- [x] Empty-scope / substring high-risk cheat: fail-closed scoping
- [x] Fake recipe certification: custom `--recipe` dry-run only
- [x] Check-only clears agent stop: **fixed** — only run/harden receipts count
- [x] Stale LCOV / bad survivorPattern: fail-closed
- [x] Proofs assert material side-effects + self smell-scan
- [x] Inert pre-push (`|| true` / `if false`) + gate-runner neuter detection
- [x] Receipt magic + full-content tree fingerprint; adopted+no-git fail-closed
- [x] Cover source-ext unmatched even when LCOV is other-language-only
- [x] prePush pinned in judge config; doctor hooksPath soft check
- [ ] Optional later: `harden-accept`, `orient`, full Battlemage required-lane wiring

## Release mechanics

1. `cargo test` green (unit + proof + demo + benchmark + cli)
2. `examples/demo` demo.sh still tells the three-arm story
3. CHANGELOG: move Unreleased → `## 0.1.0` with a date
4. Tag `v0.1.0` on the five/six-commit history
5. `cargo-dist` / GitHub release assets if installers stay in README
6. `git remote add origin …` and first push (no force on empty remote)
7. Confirm `.gitignore` still hides local harness dirs; no `docs/internal` in tree

## Explicit non-goals for 0.1.0

- Battlemage production adoption
- Cryptographic multi-party approval
- Replacing nextest / cargo-mutants / Playwright
