#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# GNU-side A/B: run each suite under /usr/local/bin/bash (5.3.0) and
# /usr/bin/bash (5.2.21) and count output drift lines.  Diagnosis aid
# for the v5->v6 ledger regression hunt (GNU version drift theory).
set -u
BASE=/mnt/d/d/repo/rubash-wt-regress/target/issue-suites/results/bash-tests-rw
OUT=/mnt/d/d/repo/rubash-wt-regress/target/issue-suites/results/gnu-ab
mkdir -p "$OUT"
for s in "$@"; do
  w="$OUT/$s"; mkdir -p "$w/tmp"
  ( cd "$BASE" && PATH="$BASE:/usr/local/bin:/usr/bin:/bin" TMPDIR="$w/tmp" THIS_SH=/bin/bash timeout -k 5 40 bash "./$s.tests" > "$w/gnu53.out" 2> "$w/gnu53.err" ) </dev/null
  ( cd "$BASE" && PATH="$BASE:/usr/local/bin:/usr/bin:/bin" TMPDIR="$w/tmp" THIS_SH=/bin/bash timeout -k 5 40 /usr/bin/bash "./$s.tests" > "$w/gnu5221.out" 2> "$w/gnu5221.err" ) </dev/null
  n=$(diff "$w/gnu53.out" "$w/gnu5221.out" | grep -c "^[<>]")
  echo "GNU-DRIFT $s $n"
done
