#!/usr/bin/env bash
set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASH_UPSTREAM_DIR="${BASH_UPSTREAM_DIR:-"$ROOT_DIR/third_party/bash"}"
BASH_TEST_DIR="$BASH_UPSTREAM_DIR/tests"
OUT_DIR="${BASH_UPSTREAM_OUT_DIR:-"$ROOT_DIR/target/bash-upstream-tests"}"
STRICT="${BASH_UPSTREAM_STRICT:-0}"

mkdir -p "$OUT_DIR/logs"

if [[ ! -d "$BASH_TEST_DIR" ]]; then
  echo "Bash upstream tests not found at $BASH_TEST_DIR" >&2
  echo "Run: git submodule update --init --depth 1 third_party/bash" >&2
  exit 2
fi

cargo build --manifest-path "$ROOT_DIR/Cargo.toml" >/dev/null

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
  TOTAL=$((TOTAL + 1))
  log="$OUT_DIR/logs/$runner.log"
  workdir="$OUT_DIR/work/$runner"
  test_workdir="$workdir/tests"
  tmpdir="$workdir/tmp"
  rm -rf "$workdir"
  mkdir -p "$tmpdir"
  cp -R "$BASH_TEST_DIR" "$test_workdir"

  (
    cd "$test_workdir"
    env \
      THIS_SH="$SHELL_BIN" \
      BUILD_DIR="$BASH_UPSTREAM_DIR" \
      BASH_TSTOUT="$tmpdir/bashtst.out" \
      TMPDIR="$tmpdir" \
      PATH="$BASH_TEST_DIR:$PATH" \
      sh "./$runner"
  ) >"$log" 2>&1
  status=$?

  if [[ "$status" -eq 0 && ! -s "$log" ]]; then
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
