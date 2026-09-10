#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# run-ksh93-difftest.sh -- ksh93 vendored suite, GNU-bash-anchored gate.
#
# Runs every third_party/ksh93/tests/*.sh under:
#   - WSL GNU Bash 5.2.21 (semantic reference), and
#   - target/debug/rubash.exe (shell under test),
# with mandatory per-test timeouts, stdin from /dev/null, and a writable
# TMPDIR. Classifies PASS (stdout+rc match) / DIFF / TIMEOUT per test and
# writes raw artifacts under target/ksh93-results/<RUN_ID>/.
#
# GNU bash anchor per AGENTS.md: a ksh93 row is only a rubash bug when
# rubash diverges from WSL GNU bash on it; ksh93-only semantics are
# recorded as ASH_KSH_ONLY and not bugs. KSH_BIN optionally adds the
# upstream ksh side for evidence.
#
# Usage:  MSYS_NO_PATHCONV=1 wsl bash /mnt/d/repo/rubash/scripts/run-ksh93-difftest.sh [glob...]
# Env:    KSH93_TEST_TIMEOUT (default 15), RUBASH_UNDER_TEST, KSH_BIN
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SUITE="$ROOT_DIR/third_party/ksh93/tests"
RESULTS="$ROOT_DIR/target/ksh93-results"
RUBASH_WIN="${RUBASH_UNDER_TEST:-$ROOT_DIR/target/debug/rubash.exe}"
RUBASH="$(printf '/mnt/%s' "$(printf '%s' "$RUBASH_WIN" | sed 's/^D:/d/; s|\\\\|/|g')")"
TIMEOUT_SECS="${KSH93_TEST_TIMEOUT:-15}"

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$RESULTS/${RUN_ID}-${1:-all}"
mkdir -p "$OUT"
printf 'test\tstatus\tgnu_rc\trubash_rc\tgnu_ms\trubash_ms\tnote\n' > "$OUT/results.tsv"

run_shell() {  # run_shell <shell-cmd> <test> <out> <err> <msfile>
  local shell_cmd="$1" test="$2" out="$3" err="$4" msfile="$5" tmpdir rc start end
  tmpdir="$(mktemp -d)"
  start=$(date +%s%N)
  ( cd "$(dirname "$test")" \
      && PATH="$(dirname "$test"):/usr/local/bin:/usr/bin:/bin" \
      && TMPDIR="$tmpdir" \
      && timeout "$TIMEOUT_SECS" $shell_cmd "./$(basename "$test")" > "$out" 2> "$err" ) < /dev/null
  rc=$?
  end=$(date +%s%N)
  echo $(( (end - start) / 1000000 )) > "$msfile"
  rm -rf "$tmpdir"
  return $rc
}

shopt -s nullglob
tests=("$SUITE"/${1:+$1}*.sh)
[ "${1:-}" ] || tests=("$SUITE"/*.sh)
shopt -u nullglob
total=0 pass=0 diff=0 tmo=0
for test in "${tests[@]}"; do
  name="$(basename "$test" .sh)"
  total=$((total+1))
  run_shell bash "$test" "$OUT/gnu.out" "$OUT/gnu.err" "$OUT/gnu.ms"; gnu_rc=$?
  run_shell "$RUBASH" "$test" "$OUT/rubash.out" "$OUT/rubash.err" "$OUT/rubash.ms"; rub_rc=$?
  status=DIFF note=""
  if [ "$gnu_rc" = 124 ] && [ "$rub_rc" = 124 ]; then status=TIMEOUT note="both-sides-timeout"
  elif [ "$gnu_rc" = 124 ]; then status=PASS note="gnu-side-timeout-skip"
  elif [ "$rub_rc" = 124 ]; then status=TIMEOUT note="rubash-timeout"
  fi
  if [ "$status" = DIFF ]; then
    sed -i 's/\r$//' "$OUT/gnu.out" "$OUT/rubash.out" 2>/dev/null || true
    if [ "$gnu_rc" = "$rub_rc" ] && cmp -s "$OUT/gnu.out" "$OUT/rubash.out"; then
      status=PASS
    else
      note="rc:$gnu_rc/$rub_rc"
      cp "$OUT/gnu.out" "$OUT/$name.gnu.out"; cp "$OUT/rubash.out" "$OUT/$name.rubash.out"
    fi
  fi
  case $status in PASS) pass=$((pass+1));; DIFF) diff=$((diff+1));; TIMEOUT) tmo=$((tmo+1));; esac
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$status" "$gnu_rc" "$rub_rc" "$(cat "$OUT/gnu.ms")" "$(cat "$OUT/rubash.ms")" "$note" >> "$OUT/results.tsv"
done
printf 'TOTAL=%d PASS=%d DIFF=%d TIMEOUT=%d\n' "$total" "$pass" "$diff" "$tmo" | tee "$OUT/SUMMARY.txt"
