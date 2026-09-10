#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# true-baseline.sh variant that pins the GNU side to /usr/bin/bash 5.2.21
# (the system bash) by REMOVING /usr/local/bin from PATH.  Context: a
# bash 5.3.0 was installed at /usr/local/bin/bash on 2026-09-09 and
# silently became the GNU baseline for the default true-baseline.sh;
# the v5->v6 ledger comparison spanned that install.  This variant
# exists to A/B the GNU-version effect.  Ledger/out paths are separate
# so the two harnesses never clobber each other's artifacts.
set -u

REPO=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BASE="$REPO/target/issue-suites/results/bash-tests-rw"
OUT="$REPO/target/issue-suites/results/true-baseline-521"
RUB="$REPO/target/debug/rubash.exe"
LOG="$REPO/target/issue-suites/results/true-baseline-521-ledger.log"
TESTS_SRC="$REPO/third_party/bash/tests"

mkdir -p "$BASE"
if [ ! -f "$BASE/recho" ] && [ -d "$TESTS_SRC" ]; then
  cp -r "$TESTS_SRC/." "$BASE/"
  find "$BASE" -type f \( -name "*.tests" -o -name "run-*" -o -name "*.right" -o -name "*.sub" \) \
    -exec sh -c 'tr -d "\r" < "$1" > "$1.lf" && mv "$1.lf" "$1"' _ {} \;
fi
sync_suite() {
  [ -f "$TESTS_SRC/$1.tests" ] || return 1
  tr -d "\r" < "$TESTS_SRC/$1.tests" > "$BASE/$1.tests"
}
ensure_test_helpers() {
  local h
  for h in recho zecho; do
    if [ ! -x "$BASE/$h" ] && [ -f "$REPO/third_party/bash/support/$h.c" ]; then
      gcc -O1 -o "$BASE/$h" "$REPO/third_party/bash/support/$h.c" 2>/dev/null || true
    fi
  done
}
ensure_test_helpers

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
      THIS_SH=/usr/bin/bash timeout -k 5 40 /usr/bin/bash "./$name.tests" \
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
