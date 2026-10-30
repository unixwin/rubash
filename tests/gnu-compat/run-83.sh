#!/usr/bin/env bash
# 83-test harness: rubash vs GNU Bash over third_party/bash/tests.
#
# Modes:
#   gen    Regenerate tests/gnu-compat/upstream-rights/<name>.right from
#          WSL GNU Bash (5.3.0 as of 2026-09; historically 5.2.21) in a clean
#          env (compiled recho/zecho/printenv, CRLF-fixed test copies,
#          THIS_SH=bash). Needs WSL.
#   check  Compare rubash against the committed .right files. No WSL needed;
#          this is the fast dev/CI loop.
#   live   rubash vs live WSL GNU Bash; for debugging the baseline itself.
#
# Usage: run-83.sh <gen|check|live> [test-name ...]   (no names = all)
#
# Artifacts: target/issue-suites/results/<mode>/<name>.{rubash.out,gnu.out,diff}
# Summary:   target/issue-suites/results/<mode>/SUMMARY.txt
#
# Tests whose GNU baseline itself cannot finish on this machine are listed in
# tests/gnu-compat/GNU-TIMEOUT.txt and skipped with a note.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUBASH="$ROOT_DIR/target/debug/rubash.exe"
SRC_TESTS="$ROOT_DIR/third_party/bash/tests"
CLEAN_TESTS="$ROOT_DIR/target/upstream-tests"
RIGHT_DIR="$ROOT_DIR/tests/gnu-compat/upstream-rights"
RESULTS="$ROOT_DIR/target/issue-suites/results"
GNU_TIMEOUT_LIST="$ROOT_DIR/tests/gnu-compat/GNU-TIMEOUT.txt"
# Derive every cross-environment path from ROOT_DIR so the harness works from
# any checkout/worktree, not just /d/repo/rubash.
HELPERS_WIN="$ROOT_DIR/tests/gnu-compat/helpers-win"   # POSIX style: see notes
ROOT_WSL="$(printf '%s' "$ROOT_DIR" | sed 's|^/\([a-zA-Z]\)/|/mnt/\1/|')"  # /d/x -> /mnt/d/x
WSL_TESTS="$ROOT_WSL/target/upstream-tests"
__drv="$(printf '%s' "${ROOT_DIR:1:1}" | tr '[:lower:]' '[:upper:]')"
ROOT_WIN_FWD="${__drv}:$(printf '%s' "${ROOT_DIR:2}")"                     # /d/x -> D:/x
ROOT_WIN_BS="${ROOT_WIN_FWD//\//\\}"                                       # D:\x
WSL_ENV="/tmp/bash-test-env"
MODE="${1:?usage: run-83.sh <gen|check|live> [test-name ...]}"
shift
# Default timeout follows the authoritative live-A/B caliber: 150s. The old
# 15s default truncated hangy groups (arith/read/printf/history) mid-output
# and mis-counted them as DIFF, making check PASS counts incomparable with
# the authoritative ledger. Override with RUN83_TIMEOUT=<secs>.
TIMEOUT_SECS="${RUN83_TIMEOUT:-150}"

OUT="$RESULTS/$MODE"
mkdir -p "$OUT"

# Serialize whole invocations: every run rebuilds the shared CLEAN_TESTS copy
# (rm -rf + copy of ~700 files), so concurrent check/gen runs tear each other
# mid-run ("./X.tests: No such file or directory", truncated rubash.out).
# mkdir is atomic on the Windows filesystem WSL sees; spin-wait up to 10 min.
LOCK_DIR="$RESULTS/.invoke-lock"
lock_acquired=""
for _ in $(seq 1 300); do
  if mkdir "$LOCK_DIR" 2>/dev/null; then lock_acquired=1; break; fi
  sleep 2
done
if [ -z "$lock_acquired" ]; then
  echo "run-83.sh: lock timeout waiting for $LOCK_DIR" >&2
  exit 3
fi
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

prepare_clean_copy() {
  rm -rf "$CLEAN_TESTS"
  mkdir -p "$CLEAN_TESTS"
  for f in "$SRC_TESTS"/*; do
    [ -f "$f" ] && cp "$f" "$CLEAN_TESTS"/
  done
  find "$CLEAN_TESTS" \( -name '*.tests' -o -name '*.sub' \) -exec sed -i 's/\r$//' {} +
}

prepare_wsl_helpers() {
  # Compile the official support/ helpers for the GNU side; support/recho's
  # checked-in ELF is reused as-is when gcc output is identical enough.
  wsl bash -c "rm -rf $WSL_ENV && mkdir -p $WSL_ENV" 2>/dev/null
  for h in recho zecho printenv; do
    wsl bash -c "gcc -O1 -o $WSL_ENV/$h $ROOT_WSL/third_party/bash/support/$h.c" 2>/dev/null ||
      wsl bash -c "cp $ROOT_WSL/third_party/bash/support/$h $WSL_ENV/$h && chmod +x $WSL_ENV/$h" 2>/dev/null
  done
}

prepare_posix_root() {
  # rubash translates POSIX-absolute external arguments (/bin, /etc, /tmp,
  # ...) against the configured shell root (__RUBASH_SHELL_ROOT, see
  # src/executor/path.rs external_argument_path). Upstream tests (rsh,
  # coproc, posixexp, ...) reference /bin/sh and /etc/passwd, which exist on
  # WSL but not for a bare rubash under Git Bash. Instead of patching
  # winuxcmd per-command, give rubash the same mechanism the product uses:
  # a minimal fixture root that makes those paths real, so rubash's own
  # spawn-layer conversion handles them.
  POSIX_ROOT="$ROOT_DIR/target/issue-suites/posix-root"
  mkdir -p "$POSIX_ROOT/bin" "$POSIX_ROOT/etc" "$POSIX_ROOT/usr/bin" \
           "$POSIX_ROOT/tmp" "$POSIX_ROOT/var/tmp" "$POSIX_ROOT/home"
  # No .exe suffix: map_logical_path checks the literal spelling, and MSYS
  # cp would rename the copy to sh.exe.
  if [ ! -s "$POSIX_ROOT/bin/sh" ]; then
    cat /bin/sh > "$POSIX_ROOT/bin/sh" 2>/dev/null || true
  fi
  if [ ! -s "$POSIX_ROOT/etc/passwd" ]; then
    printf 'root:x:0:0:root:/root:/bin/bash\n' > "$POSIX_ROOT/etc/passwd"
  fi
}

run_rubash() { # $1=name  $2=outfile
  # Use `export` (not `env`) to set environment variables. The `env` command
  # is a separate process that sets vars then exec's the target; when the
  # target is a Windows .exe via WSL interop, the env vars are lost. `export`
  # sets them in the current shell so children inherit them directly.
  # WSLENV=PATH is required for the PATH prepend to survive WSL->Windows
  # interop: without it the child .exe PATH is rebuilt from the Windows
  # registry PATH (Winuxsh shim dirs first), so helpers-win only ever wins
  # for names the shim does not ship (recho/zecho) while `ls` hit the
  # shim's own incompatible WinuxCmd ls and skewed every ls-based baseline.
  (cd "$CLEAN_TESTS" && export WSLENV=PATH PATH="$HELPERS_WIN:$PATH" THIS_SH="$RUBASH" && \
    # Env hygiene: the WSL GNU side runs with a clean env, so any IDE/CI
    # variables leaking into this session (CODEBUDDY_*, BASH_FUNC_* shims,
    # BASH_ENV, ...) make the A/B asymmetric and skew varenv et al. Keep a
    # minimal whitelist, unset everything else.
    __allowed=" PATH HOME TMPDIR TEMP TMP SYSTEMROOT SYSTEMDRIVE WINDIR COMSPEC PATHEXT USERPROFILE APPDATA LOCALAPPDATA HOMEDRIVE HOMEPATH USERNAME COMPUTERNAME WSLENV THIS_SH __RUBASH_SHELL_ROOT " && \
    while IFS= read -r __v; do
      [ -n "$__v" ] || continue
      case "$__allowed" in *" $__v "*) ;; *) unset "$__v" ;; esac
    done < <(export -p | sed -n 's/^declare -x \([^= ]*\)=.*/\1/p') && \
    export __RUBASH_SHELL_ROOT="$POSIX_ROOT" && \
    timeout "$TIMEOUT_SECS" "$RUBASH" "./$1.tests") > "$2" 2>&1
}

run_gnu() { # $1=name  $2=outfile
  # Redirect INSIDE WSL: routing the two streams through wsl.exe's relay
  # onto one externally opened handle races on separate file positions and
  # mangles interleaved stdout/stderr (lost diagnostics, truncated lines,
  # stray diff-marker bytes in captured baselines). A WSL-side redirect
  # captures the true GNU byte stream. /mnt/${2#/} mirrors the hardcoded
  # /mnt/d/repo/rubash layout this script already assumes.
  timeout "$TIMEOUT_SECS" wsl bash -c \
    "cd $WSL_TESTS && export PATH=\"$WSL_ENV:\$PATH\" && export THIS_SH=bash && bash ./$1.tests > /mnt/${2#/} 2>&1"
}

in_gnu_timeout_list() {
  [ -f "$GNU_TIMEOUT_LIST" ] && grep -qxs "$1" "$GNU_TIMEOUT_LIST"
}

# ---- setup ------------------------------------------------------------------
prepare_clean_copy
prepare_posix_root
if [ "$MODE" = gen ] || [ "$MODE" = live ]; then
  prepare_wsl_helpers
fi

names=()
if [ "$#" -gt 0 ]; then
  names=("$@")
else
  for tf in "$CLEAN_TESTS"/*.tests; do
    names+=("$(basename "$tf" .tests)")
  done
fi

# ---- modes ------------------------------------------------------------------
SUMMARY="$OUT/SUMMARY.txt"
: > "$SUMMARY"
pass=0; fail=0; tmo=0; skip=0

# Path-string normalization for diffing: baselines generated under WSL bake in
# /mnt/<drive>/... spellings while Git Bash runs produce /<drive>/... and the
# shell under test may print Windows forms (D:/... or D:\...), so identical
# semantics differ byte-wise. Normalize BOTH sides to <REPO> placeholders
# before diffing; raw files stay on disk for forensics.
ROOT_WIN_FWD_SED="$(printf '%s' "$ROOT_WIN_FWD" | sed 's/[&|\\]/\\&/g')"
ROOT_WIN_BS_SED="$(printf '%s' "$ROOT_WIN_BS" | sed 's/[&|\\]/\\&/g')"
normalize_side() { # $1=in  $2=out
  sed -e 's|/mnt/\([a-zA-Z]\)/|/\1/|g' \
      -e "s|$ROOT_WSL|<REPO>|g" \
      -e "s|$ROOT_DIR|<REPO>|g" \
      -e "s|$ROOT_WIN_FWD|<REPO>|g" \
      -e "s|$ROOT_WIN_BS_SED|<REPO>|g" \
      -e 's|/mnt/\([a-zA-Z]\)/repo/rubash|<REPO>|g' \
      "$1" \
    | grep -v -e 'completion_strip_exe' \
             -e 'set +o igncr' \
             -e 'set +o restricted' \
             -e '^igncr[[:space:]]' \
             -e '^restricted[[:space:]]' \
    > "$2"
}

for name in "${names[@]}"; do
  case "$MODE" in
    gen)
      echo -n "GEN   $name ... "
      run_gnu "$name" "$OUT/$name.gnu.out"
      rc=$?
      if [ "$rc" -eq 124 ]; then
        echo "GNU-TIMEOUT (no .right written)"
        echo "$name" >> "$OUT/gnu-timeouts.txt"
        continue
      fi
      cp "$OUT/$name.gnu.out" "$RIGHT_DIR/$name.right"
      echo "ok ($(wc -l < "$RIGHT_DIR/$name.right") lines)"
      ;;
    check)
      if in_gnu_timeout_list "$name"; then
        echo "SKIP  $name (GNU baseline cannot finish here)"
        skip=$((skip + 1))
        continue
      fi
      if [ ! -s "$RIGHT_DIR/$name.right" ]; then
        echo "SKIP  $name (no .right)"
        skip=$((skip + 1))
        continue
      fi
      run_rubash "$name" "$OUT/$name.rubash.out"
      rc=$?
      if [ "$rc" -eq 124 ]; then
        echo "TIMEOUT $name"
        tmo=$((tmo + 1))
        continue
      fi
      normalize_side "$RIGHT_DIR/$name.right" "$OUT/$name.right.norm"
      normalize_side "$OUT/$name.rubash.out" "$OUT/$name.rubash.norm"
      if diff -u "$OUT/$name.right.norm" "$OUT/$name.rubash.norm" > "$OUT/$name.diff" 2>&1; then
        echo "PASS  $name"
        pass=$((pass + 1))
      else
        echo "DIFF  $name (rubash=$(wc -l < "$OUT/$name.rubash.out") right=$(wc -l < "$RIGHT_DIR/$name.right"))"
        fail=$((fail + 1))
      fi
      ;;
    live)
      run_rubash "$name" "$OUT/$name.rubash.out"
      rb_rc=$?
      run_gnu "$name" "$OUT/$name.gnu.out"
      gnu_rc=$?
      if [ "$rb_rc" -eq 124 ] || [ "$gnu_rc" -eq 124 ]; then
        echo "TIMEOUT $name (rubash_rc=$rb_rc gnu_rc=$gnu_rc)"
        tmo=$((tmo + 1))
        continue
      fi
      normalize_side "$OUT/$name.gnu.out" "$OUT/$name.gnu.norm"
      normalize_side "$OUT/$name.rubash.out" "$OUT/$name.rubash.norm"
      if diff -q "$OUT/$name.rubash.norm" "$OUT/$name.gnu.norm" >/dev/null 2>&1; then
        echo "PASS  $name"
        pass=$((pass + 1))
      else
        diff -u "$OUT/$name.gnu.norm" "$OUT/$name.rubash.norm" > "$OUT/$name.diff"
        echo "DIFF  $name (rubash=$(wc -l < "$OUT/$name.rubash.out") gnu=$(wc -l < "$OUT/$name.gnu.out"))"
        fail=$((fail + 1))
      fi
      ;;
  esac
done

echo "=== MODE=$MODE PASS=$pass DIFF=$fail TIMEOUT=$tmo SKIP=$skip ===" >> "$SUMMARY"
tail -1 "$SUMMARY"
