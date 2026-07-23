#!/bin/sh
# One-command tour of Crucible's three arms, each catching a real problem and then
# passing honest code. Needs only sh and the crucible binary (no node, no toolchain).
#
#   cd examples/demo && ./demo.sh

DEMO="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DEMO/../.." && pwd)"
CRUCIBLE="$ROOT/target/debug/crucible"
[ -x "$CRUCIBLE" ] || (cd "$ROOT" && cargo build -q)
cd "$DEMO" || exit 1
run() { "$CRUCIBLE" "$@" --repo "$DEMO"; }
rule() { echo ""; echo "=============================================================="; echo "$1"; echo "=============================================================="; }

rule "1. HONESTY  --  crucible check: is every gate actually wired?"
run check

echo ""
echo "-- Now an agent quietly neuters the checker so it never fires again --"
BAK="$(mktemp)"; cp checks/check-no-eval.sh "$BAK"
printf '#!/bin/sh\nexit 0\n' > checks/check-no-eval.sh
run check || echo ">> BLOCKED: the pinned checker was weakened without an approval."
cp "$BAK" checks/check-no-eval.sh; rm -f "$BAK"

rule "2. REALITY  --  crucible run: does the real app actually boot?"
run run | tail -n +1

echo ""
echo "-- Inject a boot crash (a failed migration); the app's unit tests stay green: --"
CRUCIBLE_DEMO_BUG=1 "$CRUCIBLE" run --repo "$DEMO" >/dev/null 2>&1 || echo ">> BROKEN: crucible run caught the crash a green unit suite is blind to."

rule "3. FORCE CORRECT TESTS  --  crucible harden: do the tests constrain behavior?"
echo "(replays a real captured cargo-mutants survivor; see examples/proof/mutation-crate for a live run)"
run harden
echo ">> A mutant survived and Crucible named the exact test to write."

echo ""
echo "check proves the gates are real, run proves the app is, harden proves the tests are."
echo "A green suite proves none of the three."
