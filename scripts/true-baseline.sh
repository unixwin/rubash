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
# RUB_OVERRIDE: point the binary-under-test at a product-layer binary
# (e.g. niu.exe) for product-layer baselines without editing this file.
RUB="${RUB_OVERRIDE:-$REPO/target/debug/rubash.exe}"
LOG="${LOG_OVERRIDE:-$REPO/target/issue-suites/results/true-baseline-ledger.log}"
TESTS_SRC="$REPO/third_party/bash/tests"

# ---- WinuxCmd 1.0.6: prefer over Git coreutils 8.32 for the Rubash side -----
# WinuxCmd 1.0.6 matches GNU coreutils 9.4 byte-for-byte; Git's 8.32 does not.
# NOTE: The Rubash side runs as a Windows process inside a WSL-launched script.
# The WSL PATH uses Linux-style /mnt/c/... paths that Windows processes cannot
# resolve directly. Rubash inherits the Windows PATH from the WSL interop layer,
# which typically resolves to Git's coreutils 8.32. This is a known harness
# limitation; od/expr output format differences from this are NOT rubash bugs.

# ---- GNU baseline is CONTRACTUAL: GNU bash 5.3.0 (owner directive) ---------
# The owner compiled and installed GNU bash 5.3.0 into /usr/local/bin

# (2026-09-09) and directed the compat target to 5.3. Resolve the GNU side
# explicitly and refuse any other version. Legacy 5.2.21 reference ledger:
# scripts/true-baseline-521.sh.
GNU_BASH=/usr/local/bin/bash
GNU_VER=$("$GNU_BASH" --version | head -1)
case "$GNU_VER" in *"version 5.3.0"*) ;; *) echo "FATAL: GNU baseline must be 5.3.0, got: $GNU_VER" >&2; exit 9 ;; esac

# ---- locale: ensure en_US.UTF-8 is available --------------------------------
# intl.tests/unicode*.sub require en_US.UTF-8 for proper multibyte char counting.
# Without it, GNU bash emits "warning: setlocale: LC_ALL: cannot change locale"
# and falls back to C locale (byte counting), producing a broken baseline that
# inflates intl diff by ~57 lines of pure environment noise. Generate the locale
# if missing (one-time, ~1s). This makes the GNU side match intl.right.
if ! locale -a 2>/dev/null | grep -qx en_US.utf8; then
  locale-gen en_US.UTF-8 >/dev/null 2>&1 || true
fi
# intl2.sub also needs de_DE.UTF-8 for LC_NUMERIC decimal separator tests
if ! locale -a 2>/dev/null | grep -qx de_DE.utf8; then
  locale-gen de_DE.UTF-8 >/dev/null 2>&1 || true
fi
export LC_ALL=en_US.UTF-8

# ---- sync: LF-normalized rw copies -----------------------------------------
# Gap-fill + CR-repair every run. The previous seed ran only when $BASE/recho
# was absent and its find(1) pattern skipped extension-less helpers
# (test-glue-functions, history.list, execscript, misc/, version*), so a BASE
# seeded early kept missing/CRLF helpers forever: suites then produced
# identical "command not found" output on both sides and false-zero diffs
# (posixpipe hid a broken |& pipeline this way). Re-syncing unconditionally
# makes the measured surface match third_party/bash/tests exactly.
mkdir -p "$BASE"
if [ -d "$TESTS_SRC" ]; then
  for f in "$TESTS_SRC"/*; do
    b=${f##*/}
    if [ -d "$f" ]; then
      if [ ! -e "$BASE/$b" ]; then
        cp -r "$f" "$BASE/$b"
        find "$BASE/$b" -type f -exec sh -c 'tr -d "\r" < "$1" > "$1.lf" && mv "$1.lf" "$1"' _ {} \;
      fi
      continue
    fi
    # recho is an ELF helper — piping it through tr destroys the binary
    # (observed: "Exec format error" poisoning comsub et al). Everything
    # else in the tests dir is text; files that file(1) reports as "data"
    # are latin1/UTF-8 text whose only CRs are CRLF checkout pollution
    # (verified: zero files contain a lone CR). Gate on ELF magic.
    is_elf() { [ "$(od -An -tx1 -N4 "$1" 2>/dev/null | tr -d ' \n')" = "7f454c46" ]; }
    if [ ! -e "$BASE/$b" ]; then
      if is_elf "$f"; then cp "$f" "$BASE/$b"; else tr -d '\r' < "$f" > "$BASE/$b"; fi
    elif ! is_elf "$BASE/$b" && grep -q "$(printf '\r')" "$BASE/$b" 2>/dev/null; then
      tr -d '\r' < "$BASE/$b" > "$BASE/$b.lf" && mv "$BASE/$b.lf" "$BASE/$b"
    fi
  done
fi
# always re-normalize requested suites (the repo file is the source of truth)
sync_suite() {
  [ -f "$TESTS_SRC/$1.tests" ] || return 1
  tr -d "\r" < "$TESTS_SRC/$1.tests" > "$BASE/$1.tests"
}

# ---- helpers: recho/zecho must exist or every GNU output truncates ---------
# (a fresh checkout has no binaries; a missing helper makes the GNU side
#  abort with "command not found", poisoning the baseline silently)
# The GNU side gets the ELF builds from support/*.c. The rubash side runs as
# a Windows process and cannot exec ELF, so it gets REAL Windows helpers
# compiled from scripts/test-helpers/*.rs — the executor no longer carries
# test-only recho/zecho emulations (removed with upstream_scripts).
ensure_test_helpers() {
  local h
  for h in recho zecho; do
    if [ ! -x "$BASE/$h" ] && [ -f "$REPO/third_party/bash/support/$h.c" ]; then
      gcc -O1 -o "$BASE/$h" "$REPO/third_party/bash/support/$h.c" 2>/dev/null || true
    fi
  done
  if command -v rustc >/dev/null 2>&1; then
    RUSTC_BUILD=rustc
  elif command -v rustc.exe >/dev/null 2>&1; then
    # WSL side reaching the Windows toolchain through interop; Windows
    # rustc needs Windows paths, so route operands through wslpath -w.
    RUSTC_BUILD=rustc.exe
  else
    RUSTC_BUILD=
  fi
  if [ -n "$RUSTC_BUILD" ]; then
    for h in recho zecho; do
      if [ ! -f "$BASE/$h.exe" ] && [ -f "$REPO/scripts/test-helpers/$h.rs" ]; then
        if [ "$RUSTC_BUILD" = rustc.exe ]; then
          "$RUSTC_BUILD" -O -o "$(wslpath -w "$BASE/$h.exe")" "$(wslpath -w "$REPO/scripts/test-helpers/$h.rs")" 2>/dev/null || true
        else
          "$RUSTC_BUILD" -O -o "$BASE/$h.exe" "$REPO/scripts/test-helpers/$h.rs" 2>/dev/null || true
        fi
      fi
    done
  fi
}
ensure_test_helpers

# ---- suite list -------------------------------------------------------------
if [ $# -eq 0 ]; then
  SUITES=$(cd "$TESTS_SRC" && ls *.tests 2>/dev/null | sed "s/[.]tests$//")
else
  SUITES="$*"
fi

# ---- /bin/sh fixture (rb side only) ----------------------------------------
# Upstream tests spawn `/bin/sh` by absolute path (jobs3.sub `sleep 4`,
# histexp `/bin/sh -c 'echo this is $0'`, errors/rsh). On Windows there is no
# /bin; executor/path.rs degrades /bin/sh|/usr/bin/sh to a `sh` found on PATH
# (the same chain /bin/bash already used). The fixture dir ships sh.exe = a
# niubash build mounted on THIS rubash worktree, so /bin/sh children run our
# own engine ($BASH -> niu) instead of an unrelated Git/WSL shell — every
# diff line then stays attributable to rubash semantics. PATH propagates to
# ${THIS_SH} sub-shells naturally; no logical-root env is touched, so `cd /`
# and dstack keep real semantics.
SHFIX="$REPO/target/sh-fixture"
NIU_SRC="${NIUBASH_BIN:-$REPO/../niubash/target/debug/niu.exe}"
mkdir -p "$SHFIX"
if [ -x "$NIU_SRC" ]; then
  cp -f "$NIU_SRC" "$SHFIX/sh.exe" 2>/dev/null || true
fi
# RB_PATH prefixes the fixture so `sh` resolves to niu before Git's usr/bin.
RB_PATH="$SHFIX"
[ -x "$SHFIX/sh.exe" ] || echo "WARN: no sh fixture; /bin/sh falls back to PATH" >&2

# WSL -> Win32 env propagation is opt-in: without a WSLENV /w entry a custom
# variable never reaches rubash.exe (verified: __RUBASH_NO_UPSTREAM_SCRIPTS
# and TMPDIR silently vanished, so past runs measured with upstream stub
# scripts ENABLED and every suite shared one Windows %TEMP% — the redir
# `to c` xN accumulation). Flags: /w share WSL->Win32, /p translate the
# value as a Linux path into its Windows form.
# MSYS=winsymlinks:nativestrict makes Git coreutils `ln -s`/`cp -s` create real
# NTFS symlinks instead of copies, matching GNU ln semantics on the rb side
# (globstar3.sub symlink traversal depends on it).
export MSYS=winsymlinks:nativestrict
# OLDPWD/wp: suites cd into the fixture dir inside a subshell that exports
# OLDPWD=<parent cwd>, so GNU's errors.tests `cd -` returns to the repo root
# and every `${THIS_SH} ./errorsN.sub` then ENOENTs. Without the /wp entry the
# variable never crosses to rubash.exe, `cd -` reports "OLDPWD not set", and
# the .sub bodies run against GNU's empty output (~115 phantom diff lines —
# verified identical on a HEAD build under the same WSLENV).
export WSLENV="__RUBASH_NO_UPSTREAM_SCRIPTS/w:TMPDIR/p:LC_ALL/w:LC_COLLATE/w:LANG/w:MSYS/w:OLDPWD/wp"

# The rubash side runs as a Windows process: the WSL PATH never reaches it
# (WSLENV is opt-in per variable). Forward a CONTROLLED PATH via PATH/p
# translation — bash-tests-rw first (the recho/zecho.exe Windows helpers),
# then the real Windows PATH in /mnt form so the toolset resolution matches
# what interop would have passed. /usr/bin entries are deliberately NOT
# forwarded: \\wsl$ UNC paths would shadow the Windows toolset and change
# od/expr formats mid-ledger.
WINPATH_AS_WSL=$(/mnt/c/Windows/System32/cmd.exe /c 'echo %PATH%' 2>/dev/null \
  | tr -d '\r' | tr ';' '\n' \
  | sed -e 's|\\|/|g' -e 's|^\([A-Za-z]\):|/mnt/\L\1|' \
  | grep -v '^$' | paste -sd:)
RUBSIDE_PATH="${RUBSIDE_PATH:-$BASE:$WINPATH_AS_WSL}"

mkdir -p "$OUT"
: > "$LOG"

# ---- env-bound diff classification -----------------------------------------
# NTFS cannot create filenames containing : * ? " < > | -- test suites that
# `touch` such names (extglob/glob a:b cases) fail CREATION on the rubash side
# while the GNU side (ext4) succeeds. That is a filesystem capability gap, not
# a shell semantic gap, so the ledger reports it separately: each suite line is
# "name <total> env=<n>", where env= is the hunk count attributable solely to
# illegal-filename tokens. Raw gnu.out/rb.out artifacts are untouched -- the
# classification is reviewable against them. A hunk is env-bound only when
# EVERY token present on one side but not the other contains an illegal
# character; any differing legal token keeps the hunk in the rubash count.
# "/" is deliberately NOT in the illegal set: diff tokens are often whole
# paths, and path-shape differences (e.g. vredir /bin/* expansion) are real
# rubash-side questions, not filename-creation failures.
count_diff_env() { # <gnu.out> <rb.out> -> prints "<total> <env_count>"
  diff "$1" "$2" 2>/dev/null | awk -v RS='' -v FS='\n' '
    {
      delete gt; delete rt; gl=0; rl=0
      for (i = 1; i <= NF; i++) {
        l = $i
        if (l ~ /^</) { gl++; c = split(substr(l, 3), t, " "); for (j = 1; j <= c; j++) gt[t[j]]++ }
        else if (l ~ /^>/) { rl++; c = split(substr(l, 3), t, " "); for (j = 1; j <= c; j++) rt[t[j]]++ }
      }
      env = 1
      for (tk in gt) { ex = gt[tk] - (rt[tk] + 0)
        if (ex > 0) { s = tk; gsub(/^[<>()\x27]+|[<>()\x27]+$/, "", s)
          if (s !~ /[:*?"<>|]/) env = 0 } }
      for (tk in rt) { ex = rt[tk] - (gt[tk] + 0)
        if (ex > 0) { s = tk; gsub(/^[<>()\x27]+|[<>()\x27]+$/, "", s)
          if (s !~ /[:*?"<>|]/) env = 0 } }
      total += gl + rl
      if (env) e += gl + rl
    }
    END { print total + 0, e + 0 }'
}
for name in $SUITES; do
  sync_suite "$name" || { echo "$name SKIP(no-source)" >> "$LOG"; continue; }
  w="$OUT/$name"; mkdir -p "$w/tmp"
  # timeout(1) without --foreground puts the command in a new process group;
  # suite pieces that touch the terminal (history4.sub's `bash --norc -i`,
  # jobs.tests `set -m`) then stop on SIGTTIN/SIGTTOU and ignore the TERM,
  # surfacing as a fake 40s SIGKILL timeout. --foreground keeps them in the
  # caller's group so the tty ops succeed. jobs.tests additionally needs >40s
  # of real wall-clock sleeps, so it gets a larger bound.
  case "$name" in
    jobs) tmo=120 ;;
    *)    tmo=40 ;;
  esac
  ( cd "$BASE" && PATH="$BASE:/usr/bin:/bin" TMPDIR="$w/tmp" \
      THIS_SH="$GNU_BASH" timeout --foreground -k 5 "$tmo" "$GNU_BASH" "./$name.tests" \
      > "$w/gnu.out" 2> "$w/gnu.err" ) < /dev/null
  echo $? > "$w/gnu.rc"
  ( cd "$BASE" && env PATH="$RUBSIDE_PATH" TMPDIR="$w/tmp" \
      WSLENV="$WSLENV:PATH/p" \
      __RUBASH_NO_UPSTREAM_SCRIPTS=1 \
      /usr/bin/timeout --foreground -k 5 "$tmo" "$RUB" "./$name.tests" \
      > "$w/rb.out" 2> "$w/rb.err" ) < /dev/null
  echo $? > "$w/rb.rc"
  read -r n envn <<EOF
$(count_diff_env "$w/gnu.out" "$w/rb.out")
EOF
  echo "$name $n env=$envn" >> "$LOG"
done
echo TRUE-DONE
