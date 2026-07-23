#!/usr/bin/env sh
# Emit LCOV for crucible cover. Requires cargo-llvm-cov + llvm-tools.
# Unit tests only: integration tests (proof/demo/benchmark) spawn crucible heavy
# arms and deadlock on the machine-wide admission slot held by cover itself.
set -e
cd "$(dirname "$0")/.."
command -v cargo-llvm-cov >/dev/null 2>&1 || {
  echo "coverage: cargo-llvm-cov not on PATH (cargo install cargo-llvm-cov)" >&2
  exit 1
}
mkdir -p target
exec cargo llvm-cov --bins --lcov --output-path target/lcov.info
