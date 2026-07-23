#!/usr/bin/env sh
# T1: the committed empirical proofs and demo must stay green.
set -e
cd "$(dirname "$0")/.."
exec cargo test -q --test proof --test demo --test benchmark
