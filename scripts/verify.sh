#!/usr/bin/env sh
# Required per-change lane for Crucible itself. Every T1 checker is invoked here.
set -e
cd "$(dirname "$0")/.."
sh checks/check-test-smells.sh
sh checks/check-proof-suite.sh
