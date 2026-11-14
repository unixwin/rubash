#!/usr/bin/env bash
# rubash#3430: run every regression-pin stem (tests/issue*.rs) in CI.
#
# The rust job used to run a hand-curated --test list (lib, marker
# goldens, a handful of named pins), so any pin stem not on the list
# never executed while CI stayed green — PR #3424's issue3406 stem ran
# 0 times under a green CI (10 pass + 1 skipping); its only execution
# evidence was the review's box legs. This step enumerates the stems
# from the tree at run time, so a new pin is covered by default; the
# only way out is an explicit, reasoned entry in
# scripts/ci-pin-exclusions.txt (pins-first stems whose fix has not
# landed yet, known-red stems). A stem that is missing from the run
# fails loudly here instead of silently never running — fail-loud
# beats fail-silent.
set -euo pipefail
cd "$(dirname "$0")/.."

EXCLUSIONS_FILE="scripts/ci-pin-exclusions.txt"

declare -a EXCLUDE=()
while IFS= read -r line || [[ -n "$line" ]]; do
  # Full-line comments and blank lines are skipped; the first token
  # of any other line is the stem, the rest is its reason.
  case "$line" in
    ''|'#'*) continue ;;
    *) EXCLUDE+=("${line%% *}") ;;
  esac
done < "$EXCLUSIONS_FILE"

is_excluded() {
  local stem="$1" e
  for e in "${EXCLUDE[@]}"; do
    [[ "$stem" == "$e" ]] && return 0
  done
  return 1
}

# A stale exclusion is an error: the file must track the tree, or a
# stem that was fixed (or renamed) would stay silently unrun.
for e in "${EXCLUDE[@]}"; do
  if [[ ! -f "tests/${e}.rs" ]]; then
    echo "ci-pin-exclusions.txt: stale entry '$e' (tests/${e}.rs not in tree)" >&2
    exit 2
  fi
done

declare -a ARGS=()
count=0
shopt -s nullglob
for f in tests/issue*.rs; do
  stem="${f#tests/}"
  stem="${stem%.rs}"
  if is_excluded "$stem"; then
    echo "EXCLUDED: $stem"
    continue
  fi
  ARGS+=(--test "$stem")
  count=$((count + 1))
done
shopt -u nullglob

if [[ "$count" -eq 0 ]]; then
  echo "no pin stems found under tests/issue*.rs" >&2
  exit 2
fi
echo "pin stems to run: $count (excluded: ${#EXCLUDE[@]})"

# --no-fail-fast: the gate still fails the job on any red stem, but a
# first red does not hide the rest of the census (nextest stops at the
# first failure by default).
cargo nextest run --profile ci --no-fail-fast "${ARGS[@]}"
