#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT="$ROOT_DIR/target/issue-suites/busybox/shell/ash_test"
OUT_DIR="${BUSYBOX_BASH_OUT:-$ROOT_DIR/target/issue-suites/results/busybox-bash-current}"
BASH_BIN="${BASH_RUNNER:-/d/Git/bin/bash.exe}"
RUBASH_BIN="${RUBASH_UNDER_TEST:-$ROOT_DIR/target/debug/rubash.exe}"
TIMEOUT_BIN="${TIMEOUT_BIN:-/d/Git/usr/bin/timeout.exe}"
TIMEOUT_SECONDS="${BUSYBOX_TEST_TIMEOUT:-30}"

mkdir -p "$OUT_DIR"
printf 'case\tstatus\tbash_rc\trubash_rc\n' > "$OUT_DIR/results.tsv"

run_one() {
  local test_file="$1" shell_bin="$2" work="$3" rc
  local test_dir test_name
  test_dir="$(dirname "$test_file")"
  test_name="$(basename "$test_file" .tests)"
  mkdir -p "$work"
  (
    cd "$test_dir"
    export THIS_SH="$shell_bin"
    export PATH="$test_dir:/d/Git/usr/bin:/d/Git/bin:$PATH"
    export TMPDIR="$work/tmp"
    mkdir -p "$TMPDIR"
    set +e
    "$TIMEOUT_BIN" "$TIMEOUT_SECONDS" "$shell_bin" "$test_file" > "$work/stdout" 2> "$work/stderr"
  )
  rc=$?
  printf '%s\n' "$rc"
}

for test_dir in "$TEST_ROOT"/ash-*; do
  [ -d "$test_dir" ] || continue
  dir_name="$(basename "$test_dir")"
  for test_file in "$test_dir"/*.tests; do
    [ -f "$test_file" ] || continue
    test_name="$(basename "$test_file" .tests)"
    rel="$dir_name/$test_name"
    work="$OUT_DIR/work/$rel"
    bash_rc="$(run_one "$test_file" "$BASH_BIN" "$work/bash")" || true
    rubash_rc="$(run_one "$test_file" "$RUBASH_BIN" "$work/rubash")" || true
    status=PASS
    if [ "$bash_rc" != "$rubash_rc" ] || ! cmp -s "$work/bash/stdout" "$work/rubash/stdout"; then
      status=DIFF
    fi
    printf '%s\t%s\t%s\t%s\n' "$rel" "$status" "$bash_rc" "$rubash_rc" >> "$OUT_DIR/results.tsv"
  done
done

awk -F '\t' 'NR > 1 { total++; count[$2]++ } END { printf "TOTAL=%d PASS=%d DIFF=%d\n", total, count["PASS"] + 0, count["DIFF"] + 0 }' "$OUT_DIR/results.tsv" | tee "$OUT_DIR/summary.txt"
