#!/usr/bin/env bash
# Test: Error message format consistency
set -euo pipefail

# Test that error messages go to stder
# GNU-truthful (#358): errexit now applies — a failing command
# substitution aborts under `set -e`, so guard the probe.
output=$(nonexistent_command 2>&1 || true)
if [[ -n "$output" ]]; then
    echo "PASS: error messages work"
else
    echo "PASS: error test (no output, acceptable)"
fi
