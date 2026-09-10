#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# run-busybox-ash-difftest.sh -- BusyBox ash vs rubash comparison gate.
#
# Runs every vendored BusyBox ash test item (third_party/busybox/tests/ash_test,
# 335 items / 17 directories, verbatim upstream BusyBox 1.36.1 shell/ash_test)
# under BOTH:
#   - busybox ash in WSL (the reference side), and
#   - target/debug/rubash.exe (Windows binary via WSL interop),
# with a MANDATORY per-item timeout, classifying each item
# PASS / DIFF / TIMEOUT / SKIP and writing raw artifacts under
# target/busybox-results/<RUN_ID>/.
#
# Per-item timeout is NOT optional: ash-heredoc/heredoc_huge.tests is a known
# P0 hang under rubash (docs/bash-compat-issues.md #26 family A) and must
# surface as TIMEOUT, never as a runner hang.
#
# stdin discipline (documented stdin-consumption leak, family B): a test
# script's "read" can consume the driver's while-loop input stream through
# inherited stdin. This driver therefore enumerates items into arrays up
# front (no while-read over a shared fd) and runs EVERY child with stdin
# from /dev/null.
#
# Env baseline: the locale scrub below mirrors upstream run-all. Exported
# WSL env does NOT reach rubash.exe through interop except via WSLENV, so
# rubash runs with the host Windows environment (its own PATH/coreutils),
# while busybox ash runs with the WSL PATH plus compiled recho/zecho/
# printenv helpers. This asymmetry is a platform artifact of the gate and
# is part of the honest classification, not a bug to mask.
#
# Usage:
#   wsl bash /mnt/d/repo/rubash/scripts/run-busybox-ash-difftest.sh [dir-glob ...]
# Env overrides:
#   BUSYBOX_TEST_TIMEOUT  per-item timeout seconds (default 10)
#   RUBASH_UNDER_TEST     rubash binary (default <root>/target/debug/rubash.exe)
#   BUSYBOX_ASH           busybox launcher, invoked as: "$BUSYBOX_ASH" ash <script>
#                         (default: busybox)
# Artifacts:
#   target/busybox-results/<RUN_ID>/results.tsv
#       columns: case, status, bb_rc, rb_rc, bb_ms, rb_ms, bb_right, note
#       (bb_right: busybox stdout vs upstream .right sanity: OK/RIGHTDIFF/NA)
#   target/busybox-results/<RUN_ID>/SUMMARY.txt   counts + run-83-style line
#   target/busybox-results/<RUN_ID>/work/<dir>__<name>/{bb,rb}.{out,err,rc}
#   target/busybox-results/<RUN_ID>/<dir>__<name>.diff (+ .stderr.diff)
#   target/busybox-results/ash_test-rw/           writable staged copy
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ "$(uname -s)" != "Linux" ]; then
  echo "run-busybox-ash-difftest.sh: must run under WSL Linux bash:" >&2
  echo "  wsl bash /mnt/d/repo/rubash/scripts/run-busybox-ash-difftest.sh" >&2
  exit 3
fi

VENDOR="$ROOT_DIR/third_party/busybox/tests/ash_test"
RESULTS="$ROOT_DIR/target/busybox-results"
RUBASH="${RUBASH_UNDER_TEST:-$ROOT_DIR/target/debug/rubash.exe}"
BUSYBOX_ASH="${BUSYBOX_ASH:-busybox}"
TIMEOUT_SECS="${BUSYBOX_TEST_TIMEOUT:-10}"

[ -d "$VENDOR" ] || { echo "vendored suite missing: $VENDOR" >&2; exit 3; }
[ -x "$RUBASH" ] || { echo "rubash binary missing: $RUBASH" >&2; exit 3; }
"$BUSYBOX_ASH" ash -c 'exit 0' 2>/dev/null || { echo "busybox ash not usable: $BUSYBOX_ASH ash" >&2; exit 3; }
command -v timeout >/dev/null 2>&1 || { echo "coreutils timeout missing" >&2; exit 3; }
command -v gcc >/dev/null 2>&1 || echo "WARN: gcc missing; helper programs (recho/zecho/printenv) unavailable for busybox side" >&2

# Serialize whole invocations (shared RW staging copy), like run-83.sh.
mkdir -p "$RESULTS"
LOCK_DIR="$RESULTS/.run-lock"
lock_acquired=""
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if mkdir "$LOCK_DIR" 2>/dev/null; then lock_acquired=1; break; fi
  sleep 2
done
if [ -z "$lock_acquired" ]; then
  echo "run-busybox-ash-difftest.sh: lock timeout waiting for $LOCK_DIR" >&2
  exit 3
fi
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="$RESULTS/$RUN_ID"
RW="$RESULTS/ash_test-rw"
mkdir -p "$RUN_DIR/work"

# Stage a writable copy: the vendored tree must stay pristine (tests write
# $0.tmp files into their own directory).
rm -rf "$RW"
cp -a "$VENDOR" "$RW"
for h in recho zecho printenv; do
  gcc -O2 -o "$RW/$h" "$RW/$h.c" 2>/dev/null || echo "WARN: could not build helper $h" >&2
done
# ./ash link: tests spawning $THIS_SH get busybox ash via argv[0] dispatch.
ln -sf "$(command -v "$BUSYBOX_ASH")" "$RW/ash" 2>/dev/null || true

# Locale scrub, identical to upstream run-all.
unset LANG LANGUAGE
unset LC_COLLATE LC_CTYPE LC_MONETARY LC_MESSAGES LC_NUMERIC LC_TIME LC_ALL

# Enumerate items into arrays -- never a while-read over a shared stdin fd.
item_dir=()
item_name=()
for dpath in "$RW"/ash-*; do
  [ -d "$dpath" ] || continue
  d="${dpath##*/}"
  if [ "$#" -gt 0 ]; then
    keep=""
    for f in "$@"; do case "$d" in $f) keep=1 ;; esac; done
    [ -n "$keep" ] || continue
  fi
  for tpath in "$dpath"/*.tests; do
    [ -f "$tpath" ] || continue
    item_dir+=("$d")
    item_name+=("${tpath##*/}")
  done
done

results="$RUN_DIR/results.tsv"
summary="$RUN_DIR/SUMMARY.txt"
printf 'case\tstatus\tbb_rc\trb_rc\tbb_ms\trb_ms\tbb_right\tnote\n' > "$results"
: > "$summary"
pass=0; diffn=0; tmo=0; skip=0; total=0

echo "BUSYBOX-ASH-DIFFTEST run_id=$RUN_ID items=${#item_dir[@]} timeout=${TIMEOUT_SECS}s"

for i in "${!item_dir[@]}"; do
  d="${item_dir[$i]}"
  n="${item_name[$i]}"
  base="${n%.tests}"
  rel="$d/$base"
  w="$RUN_DIR/work/${d}__${base}"
  mkdir -p "$w"
  total=$((total + 1))

  # --- busybox ash (reference side, Linux) ---
  t0=$(date +%s%N)
  (
    cd "$RW/$d" || exit 97
    export THIS_SH="$RW/ash"
    export PATH="$RW:$PATH"
    timeout -k 2 "$TIMEOUT_SECS" "$BUSYBOX_ASH" ash "./$n" >"$w/bb.out" 2>"$w/bb.err" </dev/null
    echo $? >"$w/bb.rc"
  )
  t1=$(date +%s%N)
  bb_ms=$(( (t1 - t0) / 1000000 ))

  # --- rubash (Windows exe via interop; host Windows environment) ---
  t0=$(date +%s%N)
  (
    cd "$RW/$d" || exit 97
    export THIS_SH="$RUBASH"
    timeout -k 2 "$TIMEOUT_SECS" "$RUBASH" "./$n" >"$w/rb.out" 2>"$w/rb.err" </dev/null
    echo $? >"$w/rb.rc"
  )
  t1=$(date +%s%N)
  rb_ms=$(( (t1 - t0) / 1000000 ))

  bb_rc="$(cat "$w/bb.rc" 2>/dev/null || true)"
  rb_rc="$(cat "$w/rb.rc" 2>/dev/null || true)"

  status=""; note=""
  if [ -z "$bb_rc" ] || [ -z "$rb_rc" ] || [ "$bb_rc" = 97 ] || [ "$rb_rc" = 97 ]; then
    status=SKIP
    note="missing run artifact"
  elif [ "$bb_rc" = 124 ] || [ "$bb_rc" = 137 ]; then
    status=TIMEOUT
    note="busybox-side timeout"
  elif [ "$rb_rc" = 124 ] || [ "$rb_rc" = 137 ]; then
    status=TIMEOUT
    note="rubash-side timeout"
  elif cmp -s "$w/bb.out" "$w/rb.out" && [ "$bb_rc" = "$rb_rc" ]; then
    status=PASS
    cmp -s "$w/bb.err" "$w/rb.err" || note="stderr-diff"
  else
    status=DIFF
    diff -u "$w/bb.out" "$w/rb.out" >"$RUN_DIR/${d}__${base}.diff" 2>&1
    cmp -s "$w/bb.err" "$w/rb.err" || {
      note="stderr-diff"
      diff -u "$w/bb.err" "$w/rb.err" >"$RUN_DIR/${d}__${base}.stderr.diff" 2>&1
    }
  fi

  # Baseline sanity: busybox stdout vs upstream .right (same fallback-line
  # scrub as upstream run-all).
  bb_right=NA
  if [ -f "$RW/$d/$base.right" ]; then
    grep -va '^ash: using fallback suid method$' "$w/bb.out" >"$w/bb.out.scrub" 2>/dev/null
    if cmp -s "$w/bb.out.scrub" "$RW/$d/$base.right"; then
      bb_right=OK
    else
      bb_right=RIGHTDIFF
    fi
  fi

  case "$status" in
    PASS) pass=$((pass + 1)) ;;
    DIFF) diffn=$((diffn + 1)) ;;
    TIMEOUT) tmo=$((tmo + 1)) ;;
    SKIP) skip=$((skip + 1)) ;;
  esac

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$rel" "$status" "${bb_rc:--}" "${rb_rc:--}" "$bb_ms" "$rb_ms" "$bb_right" "$note" >>"$results"
  printf '%-7s %s (bb_rc=%s rb_rc=%s bb_ms=%s rb_ms=%s bb_right=%s)%s\n' \
    "$status" "$rel" "${bb_rc:--}" "${rb_rc:--}" "$bb_ms" "$rb_ms" "$bb_right" "${note:+ [$note]}"
done

{
  echo "busybox: $("$BUSYBOX_ASH" 2>&1 | head -1)"
  echo "rubash:  $RUBASH"
  echo "vendor:  $VENDOR"
  echo "timeout: ${TIMEOUT_SECS}s per item; stdin: </dev/null; locale: scrubbed like run-all"
} >"$RUN_DIR/ENV.txt"

{
  echo "total=$total pass=$pass diff=$diffn timeout=$tmo skip=$skip"
  echo "=== BUSYBOX-ASH-DIFFTEST TOTAL=$total PASS=$pass DIFF=$diffn TIMEOUT=$tmo SKIP=$skip ==="
} >>"$summary"

awk -F '\t' 'NR>1 { split($1,a,"/"); dir=a[1]; tot[dir]++; if ($2!="PASS") bad[dir]++ }
  END { for (k in tot) printf "%-16s %3d non-PASS / %3d items\n", k, bad[k]+0, tot[k] }' \
  "$results" | sort >"$RUN_DIR/PER-DIR.txt"

# Latest-run convenience pointers.
cp "$results" "$RESULTS/results.tsv"
cp "$summary" "$RESULTS/SUMMARY.txt"
cp "$RUN_DIR/PER-DIR.txt" "$RESULTS/PER-DIR.txt"

cat "$summary"
exit 0
