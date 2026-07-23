#!/usr/bin/env sh
# Diff-scoped cargo-mutants for crucible harden.
# Unit tests only: integration tests spawn nested crucible heavy arms and hang
# the unmutated baseline under cargo-mutants' per-test timeout.
set -e
cd "$(dirname "$0")/.."
base="${1:-HEAD}"
diff_file="$(mktemp -t crucible-mutants.XXXXXX.patch)"
git diff --no-renames "$base" -- . \
  ':!target' ':!mutants.out' ':!mutants.out.old' \
  ':!*.patch' > "$diff_file" || true
if [ ! -s "$diff_file" ]; then
  rm -f "$diff_file"
  echo "Found 0 mutants to test"
  echo "0 mutants tested: 0 missed, 0 caught"
  exit 0
fi
set +e
cargo mutants \
  --in-diff "$diff_file" \
  --timeout 120 \
  -j 1 \
  --no-shuffle \
  --cargo-test-arg=--bins \
  2>&1
ec=$?
set -e
rm -f "$diff_file"
exit "$ec"
