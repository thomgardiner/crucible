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
- [x] Full self-adoption: Crucible dogfoods itself (check/run/harden/cover/flake)
- [ ] Optional later: `harden-accept`, `orient`, full Battlemage required-lane wiring

## Release mechanics

1. [x] `cargo test` green (unit + proof + demo + benchmark + cli) + clippy `-D warnings`
2. [x] `examples/demo` demo.sh still tells the three-arm story
3. [x] Detection benchmark: 45/45 hacks caught, 28/28 honest passed
4. [x] cargo-dist plan + `.github/workflows/release.yml` present
5. [x] `.gitignore` hides local harness dirs; `docs/internal` ignored
6. [ ] CHANGELOG: fold Unreleased into `## 0.1.0` with ship date (do at tag time)
7. [ ] `git remote add origin git@github.com:thomgardiner/crucible.git` (or HTTPS)
8. [ ] First push of `main` (no force)
9. [ ] Tag `v0.1.0` and push tag → cargo-dist builds installers on GitHub

## Explicit non-goals for 0.1.0

- Battlemage production adoption
- Cryptographic multi-party approval
- Replacing nextest / cargo-mutants / Playwright
