#!/usr/bin/env bash
# Test: Variable export to child processes
set -euo pipefail

# GNU-truthful (#358): `set -euo pipefail` now actually applies -u, so
# the runner-set RUBASH shell variable (not exported) must have a
# default here — with GNU bash the unset expansion dies identically.
RUBASH="${RUBASH:-target/debug/rubash.exe}"

export TEST_VAR="hello_world"
result=$($RUBASH -c 'echo $TEST_VAR' 2>/dev/null || target/debug/rubash.exe -c 'echo $TEST_VAR')

if [[ "$result" == "hello_world" ]]; then
    echo "PASS: variable export works"
else
    echo "FAIL: got '$result' expected 'hello_world'"
    exit 1
fi
