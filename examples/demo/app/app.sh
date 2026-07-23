#!/bin/sh
# The demo "app": boots to ready, does a checkout, and panics on startup when the
# CRUCIBLE_DEMO_BUG env var is set (stands in for a failed migration).
if [ "$1" = "checkout" ]; then echo "ORDER PLACED #1"; exit 0; fi
if [ -n "$CRUCIBLE_DEMO_BUG" ]; then echo "thread 'main' panicked: DB migration failed on boot" >&2; exit 101; fi
echo "demo-app: ready"
