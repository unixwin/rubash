#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# TRUE-baseline harness -- THE one measurement path for the 83 GNU suites.
#
# Usage (from Windows):
#   MSYS_NO_PATHCONV=1 wsl bash /mnt/d/repo/rubash/scripts/true-baseline.sh [suite...]
# With no arguments it runs all 83 suites; with arguments only those suites.
#
# Frozen methodology (do not hand-roll probes):
#   * tests are copied from third_party/bash/tests into bash-tests-rw with
#     CR stripped (the Windows checkout CRLFs them; GNU chokes on CR)
#   * GNU side: THIS_SH=$GNU_BASH so ${THIS_SH} sub-invocations run the
#   * rubash side: no THIS_SH (auto-detects via current_exe),
#     __RUBASH_NO_UPSTREAM_SCRIPTS=1 so the real executor is measured
#   * both sides run with cwd=bash-tests-rw, PATH prefixed with it (recho /
#     zecho / run-* wrappers resolve), TMPDIR per suite, stdin </dev/null,
#     timeout -k 5 40
#   * the ledger diff counts stdout only (stderr is captured separately)
set -u

REPO=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BASE="$REPO/target/issue-suites/results/bash-tests-rw"
OUT="$REPO/target/issue-suites/results/true-baseline"
RUB="$REPO/target/debug/rubash.exe"
LOG="$REPO/target/issue-suites/results/true-baseline-ledger.log"
TESTS_SRC="$REPO/third_party/bash/tests"

# ---- GNU baseline is CONTRACTUAL: GNU bash 5.3.0 (owner directive) ---------
# The owner compiled and installed GNU bash 5.3.0 into /usr/local/bin

# (2026-09-09) and directed the compat target to 5.3. Resolve the GNU side
# explicitly and refuse any other version. Legacy 5.2.21 reference ledger:
# scripts/true-baseline-521.sh.
GNU_BASH=/usr/local/bin/bash
GNU_VER=$("$GNU_BASH" --version | head -1)
case "$GNU_VER" in *"version 5.3.0"*) ;; *) echo "FATAL: GNU baseline must be 5.3.0, got: $GNU_VER" >&2; exit 9 ;; esac

# ---- sync: LF-normalized rw copies -----------------------------------------
mkdir -p "$BASE"
if [ ! -f "$BASE/recho" ] && [ -d "$TESTS_SRC" ]; then
  cp -r "$TESTS_SRC/." "$BASE/"
  find "$BASE" -type f \( -name "*.tests" -o -name "run-*" -o -name "*.right" -o -name "*.sub" \) \
    -exec sh -c 'tr -d "\r" < "$1" > "$1.lf" && mv "$1.lf" "$1"' _ {} \;
fi
# always re-normalize requested suites (the repo file is the source of truth)
sync_suite() {
  [ -f "$TESTS_SRC/$1.tests" ] || return 1
  tr -d "\r" < "$TESTS_SRC/$1.tests" > "$BASE/$1.tests"
}

# ---- helpers: recho/zecho must exist or every GNU output truncates ---------
# (a fresh checkout has no binaries; a missing helper makes the GNU side
#  abort with "command not found", poisoning the baseline silently)
ensure_test_helpers() {
  local h
  for h in recho zecho; do
    if [ ! -x "$BASE/$h" ] && [ -f "$REPO/third_party/bash/support/$h.c" ]; then
      gcc -O1 -o "$BASE/$h" "$REPO/third_party/bash/support/$h.c" 2>/dev/null || true
    fi
  done
}
ensure_test_helpers

# ---- suite list -------------------------------------------------------------
if [ $# -eq 0 ]; then
  SUITES=$(cd "$TESTS_SRC" && ls *.tests 2>/dev/null | sed "s/[.]tests$//")
else
  SUITES="$*"
fi

mkdir -p "$OUT"
: > "$LOG"
for name in $SUITES; do
  sync_suite "$name" || { echo "$name SKIP(no-source)" >> "$LOG"; continue; }
  w="$OUT/$name"; mkdir -p "$w/tmp"
  ( cd "$BASE" && PATH="$BASE:/usr/bin:/bin" TMPDIR="$w/tmp" \
      THIS_SH="$GNU_BASH" timeout -k 5 40 "$GNU_BASH" "./$name.tests" \
      > "$w/gnu.out" 2> "$w/gnu.err" ) < /dev/null
  echo $? > "$w/gnu.rc"
  ( cd "$BASE" && PATH="$BASE:/usr/bin:/bin" TMPDIR="$w/tmp" \
      __RUBASH_NO_UPSTREAM_SCRIPTS=1 timeout -k 5 40 "$RUB" "./$name.tests" \
      > "$w/rb.out" 2> "$w/rb.err" ) < /dev/null
  echo $? > "$w/rb.rc"
  n=$(diff "$w/gnu.out" "$w/rb.out" 2>/dev/null | grep -c "^[<>]")
  echo "$name $n" >> "$LOG"
done
echo TRUE-DONE
