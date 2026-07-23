#!/bin/sh
# A real gate: no eval() in app code. Exits non-zero (and names the file) if found.
if grep -rn 'eval(' app/ 2>/dev/null; then echo "gate: eval() found in app code" >&2; exit 1; fi
exit 0
