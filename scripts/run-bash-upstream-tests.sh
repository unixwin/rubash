#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -euo pipefail

# Preserve the caller's toolchain. Do not bake a developer-specific Cargo path
# into a compatibility runner; CI and Windows hosts provide different paths.
export PATH="${PATH:-/usr/bin:/bin}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASH_UPSTREAM_DIR="$ROOT_DIR/third_party/bash"
BASH_TEST_DIR="$BASH_UPSTREAM_DIR/tests"
OUT_DIR="$ROOT_DIR/target/bash-upstream-tests"
STRICT="${BASH_UPSTREAM_STRICT:-0}"
TIMEOUT_BIN="${BASH_UPSTREAM_TIMEOUT_BIN:-timeout}"
TIMEOUT_SECONDS="${BASH_UPSTREAM_TIMEOUT:-60}"
TIMEOUT_KILL_AFTER="${BASH_UPSTREAM_KILL_AFTER:-5}"
RUN_ID="${BASH_UPSTREAM_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_OUT_DIR="$ROOT_DIR/target/issue-suites/results/bash-upstream-tests/$RUN_ID"

if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9_.-]+$ ]]; then
  echo "BASH_UPSTREAM_RUN_ID contains unsafe filename characters: $RUN_ID" >&2
  exit 125
fi

validate_timeout() {
  [[ "$TIMEOUT_SECONDS" =~ ^[1-9][0-9]*([.][0-9]+)?$ ]] || {
    echo "BASH_UPSTREAM_TIMEOUT must be a positive number of seconds: $TIMEOUT_SECONDS" >&2
    exit 125
  }
  [[ "$TIMEOUT_KILL_AFTER" =~ ^[1-9][0-9]*([.][0-9]+)?$ ]] || {
    echo "BASH_UPSTREAM_KILL_AFTER must be a positive number of seconds: $TIMEOUT_KILL_AFTER" >&2
    exit 125
  }
  command -v "$TIMEOUT_BIN" >/dev/null 2>&1 || {
    echo "Bash upstream runner requires a GNU timeout binary: $TIMEOUT_BIN" >&2
    exit 125
  }
  set +e
  "$TIMEOUT_BIN" 1 sh -c 'sleep 2' >/dev/null 2>&1
  local timeout_rc=$?
  set -e
  [[ "$timeout_rc" -eq 124 ]] || {
    echo "Bash upstream runner rejected timeout implementation $TIMEOUT_BIN (expected rc 124, got $timeout_rc)" >&2
    exit 125
  }
}

validate_timeout

real_path() {
  local resolved
  if command -v realpath >/dev/null 2>&1; then
    resolved="$(realpath -m "$1")"
  else
    resolved="$(cd "$(dirname "$1")" && printf '%s/%s\n' "$PWD" "$(basename "$1")")"
  fi

  normalize_real_path "$resolved"
}

normalize_real_path() {
  local path="${1//\\//}"
  if [[ "$path" =~ ^/([a-zA-Z])(/.*)?$ ]]; then
    local drive="${BASH_REMATCH[1]^^}"
    local rest="${BASH_REMATCH[2]:-/}"
    printf '%s:%s\n' "$drive" "$rest"
    return
  fi
  printf '%s\n' "$path"
}

die() {
  echo "$*" >&2
  exit 2
}

is_under_dir() {
  local child="$1"
  local parent="$2"

  child="${child%/}"
  parent="${parent%/}"
  [[ "$child" == "$parent"/* ]]
}

assert_under_dir() {
  local child="$1"
  local parent="$2"
  local label="$3"

  if ! is_under_dir "$child" "$parent"; then
    die "Refusing unsafe $label outside $parent: $child"
  fi
}

ROOT_REAL="$(real_path "$ROOT_DIR")"
ROOT_REAL="${ROOT_REAL%/}"
HOME_REAL="$(real_path "${HOME:-}")"

case "$ROOT_REAL" in
  ""|"/"|"$HOME_REAL"|"$HOME_REAL/Desktop"|"$HOME_REAL/Downloads"|"$HOME_REAL/Documents")
    die "Refusing unsafe repository root for Bash upstream tests: $ROOT_REAL"
    ;;
esac

[[ -f "$ROOT_REAL/Cargo.toml" ]] || die "Refusing to run outside rubash repo: missing Cargo.toml at $ROOT_REAL"
[[ -f "$ROOT_REAL/scripts/run-bash-upstream-tests.sh" ]] || die "Refusing to run outside rubash repo: missing runner script at $ROOT_REAL"
[[ -d "$ROOT_REAL/third_party/bash/tests" ]] || die "Refusing to run outside rubash repo: missing Bash tests at $ROOT_REAL/third_party/bash/tests"

OUT_REAL="$(real_path "$OUT_DIR")"
WORK_ROOT="$OUT_DIR/work"
WORK_ROOT_REAL="$(real_path "$WORK_ROOT")"

mkdir -p "$OUT_DIR/logs" "$RAW_OUT_DIR"

refuse_unsafe_dir() {
  local dir="$1"
  local real
  real="$(real_path "$dir")"

  assert_under_dir "$real" "$WORK_ROOT_REAL" "Bash upstream test directory"

  case "$real" in
    ""|"/"|"$HOME_REAL"|"$ROOT_REAL"|"$OUT_REAL")
      die "Refusing unsafe Bash upstream test directory: $real"
      ;;
  esac
}

safe_rm_rf() {
  local target="$1"
  local real
  real="$(real_path "$target")"

  assert_under_dir "$real" "$WORK_ROOT_REAL" "delete target"

  case "$real" in
    ""|"/"|"$HOME_REAL"|"$ROOT_REAL"|"$OUT_REAL"|"$WORK_ROOT_REAL")
      die "Refusing unsafe recursive delete target: $real"
      ;;
  esac

  rm -rf -- "$target"
}

if [[ ! -d "$BASH_TEST_DIR" ]]; then
  echo "Bash upstream tests not found at $BASH_TEST_DIR" >&2
  echo "Run: git submodule update --init --depth 1 third_party/bash" >&2
  exit 2
fi

if ! cargo build --manifest-path "$ROOT_DIR/Cargo.toml" >/dev/null; then
  echo "Failed to build rubash before running Bash upstream tests" >&2
  exit 2
fi

SHELL_BIN="$ROOT_DIR/target/debug/rubash"
if [[ -x "$SHELL_BIN.exe" ]]; then
  SHELL_BIN="$SHELL_BIN.exe"
fi

if [[ ! -x "$SHELL_BIN" ]]; then
  echo "Built shell is not executable: $SHELL_BIN" >&2
  exit 2
fi

mapfile -t RUNNERS < <(
  find "$BASH_TEST_DIR" -maxdepth 1 -type f -name 'run-*' \
    ! -name 'run-all' \
    ! -name 'run-gprof' \
    -printf '%f\n' | sort
)

if [[ "$#" -gt 0 ]]; then
  RUNNERS=("$@")
fi

TOTAL=0
PASS=0
FAIL=0
TIMEOUT_FAIL=0

RESULTS_TSV="$OUT_DIR/results.tsv"
SUMMARY_MD="$OUT_DIR/summary.md"

printf "test\tstatus\texit_code\tlog\traw_artifacts\n" > "$RESULTS_TSV"

for runner in "${RUNNERS[@]}"; do
  if [[ "$runner" == */* || "$runner" == *\\* ]]; then
    echo "Refusing runner name with path separators: $runner" >&2
    exit 2
  fi

  TOTAL=$((TOTAL + 1))
  log="$OUT_DIR/logs/$runner.log"
  workdir="$OUT_DIR/work/$runner"
  test_workdir="$workdir/tests"
  expected_dir="$workdir/expected"
  tmpdir="$workdir/tmp"
  test_home="$workdir/home"
  guard_bin="$workdir/guard-bin"
  shell_wrapper="$workdir/rubash-wrapper"
  refuse_unsafe_dir "$workdir"
  safe_rm_rf "$workdir"
  mkdir -p "$tmpdir" "$test_home" "$guard_bin" "$expected_dir"
  cp -R "$BASH_TEST_DIR" "$test_workdir"
  cp "$BASH_TEST_DIR"/*.right "$expected_dir"/
  # Normalize expected output line endings for Windows worktrees. The upstream
  # Bash tests compare byte-for-byte, while Rubash writes LF on all platforms.
  # TODO(tests/redir.c): replace this harness normalization once the test
  # workspace checkout is forced to LF independent of host git attributes.
  sed -i 's/\r$//' "$expected_dir"/*.right "$test_workdir"/*.right
  # The copied driver must expand BASH_TSTOUT at test runtime, not here.
  # shellcheck disable=SC2016
  sed -i 's@^TEST_FILE="/tmp/${TEST_NAME}\.check"$@TEST_FILE="${BASH_TSTOUT}"@' \
    "$test_workdir"/run-dbg-support*
  refuse_unsafe_dir "$test_workdir"
  workdir_real="$(real_path "$workdir")"
  expected_dir_real="$(real_path "$expected_dir")"
  shell_wrapper_real="$(real_path "$shell_wrapper")"

  find "$test_workdir" -maxdepth 1 -type f -name 'run-*' -exec \
    sed -i -E "s@([[:alnum:]_.+-]+\\.right)@$expected_dir_real/\\1@g" {} +

  for guarded_cmd in rm touch mkdir cp mv ln; do
    guarded_path="$(command -v "$guarded_cmd")"
    cat >"$guard_bin/$guarded_cmd" <<EOF
#!/usr/bin/env bash
set -euo pipefail
PATH="/usr/bin:/bin:\$PATH"
normalize_real_path() {
  local path="\${1//\\\\//}"
  if [[ "\$path" =~ ^/([a-zA-Z])(/.*)?$ ]]; then
    local drive="\${BASH_REMATCH[1]^^}"
    local rest="\${BASH_REMATCH[2]:-/}"
    printf '%s:%s\n' "\$drive" "\$rest"
    return
  fi
  printf '%s\n' "\$path"
}
allowed="\$(normalize_real_path "$workdir_real")"
cwd="\$(normalize_real_path "\$(realpath -m "\$PWD")")"
case "\$cwd" in
  "\$allowed"|"\$allowed"/*) ;;
  *)
    echo "Refusing $guarded_cmd outside Bash upstream work dir: \$cwd" >&2
    echo "Allowed: \$allowed" >&2
    exit 126
    ;;
esac
after_dashdash=0
for arg in "\$@"; do
  if [[ "\$after_dashdash" -eq 0 && "\$arg" == "--" ]]; then
    after_dashdash=1
    continue
  fi
  if [[ "\$after_dashdash" -eq 0 && "\$arg" == -* ]]; then
    continue
  fi

  case "\$arg" in
    "") continue ;;
  esac

  candidate="\$(normalize_real_path "\$(realpath -m -- "\$arg")")"
  if [[ "$guarded_cmd" == "cp" && "\$candidate" == "/dev/null" ]]; then
    continue
  fi
  case "\$candidate" in
    "\$allowed"|"\$allowed"/*) ;;
    *)
      echo "Refusing $guarded_cmd path outside Bash upstream work dir: \$arg -> \$candidate" >&2
      echo "Allowed: \$allowed" >&2
      exit 126
      ;;
  esac
done
exec "$guarded_path" "\$@"
EOF
    chmod +x "$guard_bin/$guarded_cmd"
  done

  cat >"$shell_wrapper" <<EOF
#!/usr/bin/env bash
set -euo pipefail
PATH="$guard_bin:/usr/bin:/bin:\$PATH"
normalize_real_path() {
  local path="\${1//\\\\//}"
  if [[ "\$path" =~ ^/([a-zA-Z])(/.*)?$ ]]; then
    local drive="\${BASH_REMATCH[1]^^}"
    local rest="\${BASH_REMATCH[2]:-/}"
    printf '%s:%s\n' "\$drive" "\$rest"
    return
  fi
  printf '%s\n' "\$path"
}
allowed="\$(normalize_real_path "$workdir_real")"
cwd="\$(normalize_real_path "\$(realpath -m "\$PWD")")"
case "\$cwd" in
  "\$allowed"|"\$allowed"/*) ;;
  *)
    echo "Refusing to start rubash outside Bash upstream work dir: \$cwd" >&2
    echo "Allowed: \$allowed" >&2
    exit 126
    ;;
esac
export HOME="$test_home"
export TMPDIR="$tmpdir"
exec "$SHELL_BIN" "\$@"
EOF
  chmod +x "$shell_wrapper"

  set +e
  (
    cd "$test_workdir"
    refuse_unsafe_dir "$PWD"
    # Some upstream .tests invoke `./bash` directly (e.g. dbg-support); provide
    # the rubash binary under that name like the Bash build would.
    cp "$SHELL_BIN" ./bash
    env \
      HOME="$test_home" \
      THIS_SH="$shell_wrapper" \
      BUILD_DIR="$BASH_UPSTREAM_DIR" \
      BASH_TSTOUT="$tmpdir/bashtst.out" \
      TMPDIR="$tmpdir" \
      PATH="$guard_bin:$BASH_TEST_DIR:$PATH" \
      "$TIMEOUT_BIN" --kill-after="$TIMEOUT_KILL_AFTER" "$TIMEOUT_SECONDS" sh "./$runner"
  ) >"$log" 2>&1
  status=$?
  set -e
  if [[ "$status" -eq 124 ]]; then
    TIMEOUT_FAIL=$((TIMEOUT_FAIL + 1))
  fi

  unexpected_log="$OUT_DIR/logs/$runner.unexpected.log"
  grep -v -x \
    -e 'declare -r SHELLOPTS="braceexpand:hashall:interactive-comments"' \
    -e "Testing $shell_wrapper" \
    -e "Testing $shell_wrapper_real" \
    -e 'version: .*' \
    -e 'HOSTTYPE = .*' \
    -e 'OSTYPE = .*' \
    -e 'MACHTYPE = .*' \
    -e 'Any output from any test, unless otherwise noted, indicates a possible anomaly' \
    -e 'run-comsub-eof' \
    -e 'run-comsub-posix' \
    -e 'run-dollars' \
    -e 'run-dynvar' \
    -e 'run-execscript' \
    -e 'run-func' \
    -e 'run-getopts' \
    -e 'run-heredoc' \
    -e 'run-ifs-posix' \
    -e 'run-input-test' \
    -e 'run-invert' \
    -e 'run-iquote' \
    -e 'run-more-exp' \
    -e 'run-nquote' \
    -e 'run-posix2' \
    -e 'run-posixpat' \
    -e 'run-posixpipe' \
    -e 'run-precedence' \
    -e 'run-quote' \
    -e 'run-read' \
    -e 'run-rhs-exp' \
    -e 'run-strip' \
    -e 'run-tilde' \
    -e 'run-type' \
    -e 'warning: some of these tests may fail if process substitution has not' \
    -e 'warning: all of these tests will fail if process substitution has not' \
    -e 'warning: two of these tests will fail if your OS does not support' \
    -e 'warning: named pipes or the /dev/fd filesystem.  If the tests of the' \
    -e 'warning: process substitution mechanism fail, please do not consider' \
    -e 'warning: this a test failure' \
    -e 'warning: some of these tests will fail if you do not have UTF-8' \
    -e 'warning: locales installed on your system' \
    -e 'warning: been compiled into the shell or if the OS does not provide' \
    -e 'warning: FIFOs or /dev/fd. Some tests may fail if the OS does not' \
    -e 'warning: provide FIFOs.' \
    -e 'warning: /dev/fd.' \
    -e 'warning: if you have exported functions defined in your environment,' \
    -e 'warning: they may show up as diff output.' \
    -e 'warning: if you have exported variables beginning with the string _Q,' \
    -e 'warning: diff output may be generated.  If so, please do not consider' \
    -e 'warning: if so, please do not consider this a test failure' \
    -e 'warning: the text of a system error message may vary between systems and' \
    -e 'warning: UNIX versions number signals differently.' \
    -e 'warning: UNIX versions number signals and schedule processes differently.' \
    -e 'warning: If output differing only in line numbers is produced, please' \
    -e 'warning: do not consider this a test failure.' \
    -e 'warning: please do not consider output differing only in the amount of' \
    -e 'warning: white space to be an error.' \
    -e "warning: if the text of the error messages concerning \`notthere' or" \
    -e "warning: \`/tmp/bash-notthere' not being found or \`/' being a directory" \
    -e "warning: if the text of an error message concerning \`redir1.*' not being" \
    -e 'warning: found or messages concerning bad file descriptors produce diff' \
    -e 'warning: output, please do not consider it a test failure' \
    -e 'warning: produce diff output, please do not consider this a test failure' \
    -e 'warning: if diff output differing only in the location of the bash' \
    -e 'warning: binary appears, please do not consider this a test failure' \
    -e 'warning: all of these tests will fail if arrays have not' \
    -e 'warning: several of these tests will fail if arrays have not' \
    -e 'warning: some of these tests will fail if arrays have not' \
    -e 'warning: been compiled into the shell' \
    -e 'warning: been compiled into the shell.' \
    -e 'warning: the BASH_ARGC and BASH_ARGV tests will fail if debugging support' \
    -e 'warning: has not been compiled into the shell' \
    -e 'warning: some of these tests may fail if job control has not been compiled' \
    -e 'warning: into the shell' \
    -e 'warning: all of these tests will fail if history has not been compiled' \
    -e 'warning: there may be a message regarding a cat process dying due to a' \
    -e 'warning: SIGHUP.  Please disregard.' \
    -e 'warning: all of these tests will fail if the conditional command has not' \
    -e 'warning: some of these tests will fail if extended pattern matching has not' \
    -e 'warning: the text of system error messages may vary between systems and' \
    -e 'warning: produce diff output.' \
    "$log" > "$unexpected_log" || true

  raw_runner_dir="$RAW_OUT_DIR/$runner"
  mkdir -p "$raw_runner_dir"
  cp "$log" "$raw_runner_dir/runner.log"
  cp "$unexpected_log" "$raw_runner_dir/runner.unexpected.log"
  cp -R "$test_workdir" "$raw_runner_dir/tests"
  cp -R "$expected_dir" "$raw_runner_dir/expected"
  cp -R "$tmpdir" "$raw_runner_dir/tmp"

  if [[ "$status" -eq 0 && ! -s "$unexpected_log" ]]; then
    PASS=$((PASS + 1))
    printf "%s\tPASS\t%s\t%s\t%s\n" "$runner" "$status" "$log" "$raw_runner_dir" >> "$RESULTS_TSV"
    printf "PASS %s (raw %s)\n" "$runner" "$raw_runner_dir"
  else
    FAIL=$((FAIL + 1))
    printf "%s\tFAIL\t%s\t%s\t%s\n" "$runner" "$status" "$log" "$raw_runner_dir" >> "$RESULTS_TSV"
    printf "FAIL %s (exit %s, log %s, raw %s)\n" "$runner" "$status" "$log" "$raw_runner_dir"
  fi
done

{
  echo "# Bash Upstream Test Progress"
  echo
  echo "- Total: $TOTAL"
  echo "- Passed: $PASS"
  echo "- Failed: $FAIL"
  echo "- Timed out: $TIMEOUT_FAIL"
  if [[ "$TOTAL" -gt 0 ]]; then
    awk -v pass="$PASS" -v total="$TOTAL" 'BEGIN { printf "- Pass rate: %.2f%%\n", (pass * 100.0) / total }'
  else
    echo "- Pass rate: 0.00%"
  fi
  echo
  echo "Results file: \`$RESULTS_TSV\`"
  echo
  echo "## Failures"
  echo
  awk -F '\t' 'NR > 1 && $2 == "FAIL" { printf "- `%s` exit `%s`, log `%s`, raw `%s`\n", $1, $3, $4, $5 }' "$RESULTS_TSV"
} > "$SUMMARY_MD"

cat "$SUMMARY_MD"

if [[ "$TIMEOUT_FAIL" -gt 0 ]]; then
  exit 1
fi

if [[ "$STRICT" == "1" && "$FAIL" -gt 0 ]]; then
  exit 1
fi

exit 0
