#!/usr/bin/env sh
# T1: the anti-reward-hacking tool's own suite must not contain test-gaming smells.
set -e
cd "$(dirname "$0")/.."
if command -v crucible >/dev/null 2>&1; then
  exec crucible test-smells tests src
fi
# Fallback when the installed binary is missing: use the workspace build.
if [ -x ./target/debug/crucible ]; then
  exec ./target/debug/crucible test-smells tests src
fi
cargo run -q -- test-smells tests src
