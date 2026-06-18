#!/usr/bin/env bash
set -euo pipefail

PATH="/c/Users/caomengxuan/.cargo/bin:/usr/bin:/bin:$PATH"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASH_UPSTREAM_DIR="$ROOT_DIR/third_party/bash"
BASH_TEST_DIR="$BASH_UPSTREAM_DIR/tests"
OUT_DIR="$ROOT_DIR/target/bash-upstream-tests"
STRICT="${BASH_UPSTREAM_STRICT:-0}"

real_path() {
  if command -v realpath >/dev/null 2>&1; then
    realpath -m "$1"
  else
    (cd "$(dirname "$1")" && printf '%s/%s\n' "$PWD" "$(basename "$1")")
  fi
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

mkdir -p "$OUT_DIR/logs"

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
    ! -name 'run-minimal' \
    ! -name 'run-gprof' \
    -printf '%f\n' | sort
)

if [[ "$#" -gt 0 ]]; then
  RUNNERS=("$@")
fi

TOTAL=0
PASS=0
FAIL=0

RESULTS_TSV="$OUT_DIR/results.tsv"
SUMMARY_MD="$OUT_DIR/summary.md"

printf "test\tstatus\texit_code\tlog\n" > "$RESULTS_TSV"

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
  sed -i 's@^TEST_FILE="/tmp/${TEST_NAME}\.check"$@TEST_FILE="${BASH_TSTOUT}"@' \
    "$test_workdir"/run-dbg-support*
  refuse_unsafe_dir "$test_workdir"
  workdir_real="$(real_path "$workdir")"
  expected_dir_real="$(real_path "$expected_dir")"

  find "$test_workdir" -maxdepth 1 -type f -name 'run-*' -exec \
    sed -i -E "s@([[:alnum:]_.+-]+\\.right)@$expected_dir_real/\\1@g" {} +

  for guarded_cmd in rm touch mkdir cp mv ln; do
    guarded_path="$(command -v "$guarded_cmd")"
    cat >"$guard_bin/$guarded_cmd" <<EOF
#!/usr/bin/env bash
set -euo pipefail
PATH="/usr/bin:/bin:\$PATH"
allowed="$workdir_real"
cwd="\$(realpath -m "\$PWD")"
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

  candidate="\$(realpath -m -- "\$arg")"
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
allowed="$workdir_real"
cwd="\$(realpath -m "\$PWD")"
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
    env \
      HOME="$test_home" \
      THIS_SH="$shell_wrapper" \
      BUILD_DIR="$BASH_UPSTREAM_DIR" \
      BASH_TSTOUT="$tmpdir/bashtst.out" \
      TMPDIR="$tmpdir" \
      PATH="$guard_bin:$BASH_TEST_DIR:$PATH" \
      sh "./$runner"
  ) >"$log" 2>&1
  status=$?
  set -e

  unexpected_log="$OUT_DIR/logs/$runner.unexpected.log"
  grep -v -x \
    -e 'warning: some of these tests may fail if process substitution has not' \
    -e 'warning: some of these tests will fail if you do not have UTF-8' \
    -e 'warning: locales installed on your system' \
    -e 'warning: been compiled into the shell or if the OS does not provide' \
    -e 'warning: /dev/fd.' \
    -e 'warning: if you have exported functions defined in your environment,' \
    -e 'warning: they may show up as diff output.' \
    -e 'warning: if so, please do not consider this a test failure' \
    -e 'warning: the text of a system error message may vary between systems and' \
    -e 'warning: UNIX versions number signals differently.' \
    -e 'warning: If output differing only in line numbers is produced, please' \
    -e 'warning: do not consider this a test failure.' \
    -e "warning: if the text of the error messages concerning \`notthere' or" \
    -e "warning: \`/tmp/bash-notthere' not being found or \`/' being a directory" \
    -e 'warning: produce diff output, please do not consider this a test failure' \
    -e 'warning: if diff output differing only in the location of the bash' \
    -e 'warning: binary appears, please do not consider this a test failure' \
    -e 'warning: all of these tests will fail if arrays have not' \
    -e 'warning: several of these tests will fail if arrays have not' \
    -e 'warning: been compiled into the shell' \
    -e 'warning: been compiled into the shell.' \
    -e 'warning: the BASH_ARGC and BASH_ARGV tests will fail if debugging support' \
    -e 'warning: has not been compiled into the shell' \
    -e 'warning: all of these tests will fail if the conditional command has not' \
    -e 'warning: some of these tests will fail if extended pattern matching has not' \
    -e 'warning: the text of system error messages may vary between systems and' \
    -e 'warning: produce diff output.' \
    "$log" > "$unexpected_log" || true

  if [[ "$status" -eq 0 && ! -s "$unexpected_log" ]]; then
    PASS=$((PASS + 1))
    printf "%s\tPASS\t%s\t%s\n" "$runner" "$status" "$log" >> "$RESULTS_TSV"
    printf "PASS %s\n" "$runner"
  else
    FAIL=$((FAIL + 1))
    printf "%s\tFAIL\t%s\t%s\n" "$runner" "$status" "$log" >> "$RESULTS_TSV"
    printf "FAIL %s (exit %s, log %s)\n" "$runner" "$status" "$log"
  fi
done

{
  echo "# Bash Upstream Test Progress"
  echo
  echo "- Total: $TOTAL"
  echo "- Passed: $PASS"
  echo "- Failed: $FAIL"
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
  awk -F '\t' 'NR > 1 && $2 == "FAIL" { printf "- `%s` exit `%s`, log `%s`\n", $1, $3, $4 }' "$RESULTS_TSV"
} > "$SUMMARY_MD"

cat "$SUMMARY_MD"

if [[ "$STRICT" == "1" && "$FAIL" -gt 0 ]]; then
  exit 1
fi

exit 0
