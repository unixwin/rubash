#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -u
set -o pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tests_dir="${BASH_LEDGER_TESTS_DIR:-$repo/third_party/bash/tests}"
out="${BASH_LEDGER_OUT:-$repo/target/issue-suites/results/bash-ledger-$(date -u +%Y%m%dT%H%M%SZ)}"
gnu="${BASH_LEDGER_GNU_BASH:-D:/Git/bin/bash.exe}"
rubash="${BASH_LEDGER_RUBASH:-$repo/target/debug/rubash.exe}"
timeout_bin="${BASH_LEDGER_TIMEOUT_BIN:-D:/Git/usr/bin/timeout.exe}"
timeout_seconds="${BASH_LEDGER_TIMEOUT_SECONDS:-30}"
test_filter="${BASH_LEDGER_TEST_FILTER:-}"

mkdir -p "$out/work/base/bash" "$out/work/base/rubash" "$out/work/results"
commit="$(git -C "$repo" rev-parse HEAD)"
printf 'commit=%s\nGNU_Bash=%s\nRubash=%s\ntimeout=%s\nnormalization=CRLF_to_LF\nfilter=%s\n' \
  "$commit" "$gnu" "$rubash" "$timeout_seconds" "$test_filter" > "$out/manifest.txt"
printf 'test\tstatus\tbash_rc\trubash_rc\n' > "$out/results.tsv"

# Copy robustly: some Windows shims do not honor `cp -R dir/.`; populate
# each base only when empty so a pre-staged tree is kept.
if [ -z "$(ls -A "$out/work/base/bash" 2>/dev/null)" ]; then
  (cd "$tests_dir" && cp -R * "$out/work/base/bash/") || cp -R "$tests_dir"/. "$out/work/base/bash/"
fi
if [ -z "$(ls -A "$out/work/base/rubash" 2>/dev/null)" ]; then
  (cd "$tests_dir" && cp -R * "$out/work/base/rubash/") || cp -R "$tests_dir"/. "$out/work/base/rubash/"
fi
find "$out/work/base/bash" "$out/work/base/rubash" -type f -exec sed -i 's/\r$//' {} +
printf '#!/usr/bin/env bash\nexec "%s" "\$@"\n' "$gnu" > "$out/work/base/bash/bash"
printf '#!/usr/bin/env bash\nexec "%s" "\$@"\n' "$rubash" > "$out/work/base/rubash/bash"
chmod +x "$out/work/base/bash/bash" "$out/work/base/rubash/bash"

for src in "$tests_dir"/*.tests; do
  name="$(basename "$src" .tests)"
  if [[ -n "$test_filter" && ",${test_filter}," != *",${name},"* ]]; then
    continue
  fi
  result="$out/work/results/$name"
  mkdir -p "$result"
  (cd "$out/work/base/bash" && "$timeout_bin" "$timeout_seconds" "$gnu" "./$name.tests" >"$result/bash.stdout" 2>"$result/bash.stderr")
  brc=$?
  (cd "$out/work/base/rubash" && "$timeout_bin" "$timeout_seconds" "$rubash" "./$name.tests" >"$result/rubash.stdout" 2>"$result/rubash.stderr")
  rrc=$?
  status=PASS
  # Normalize CRLF to LF before comparison
  sed -i 's/\r$//' "$result/bash.stdout" "$result/rubash.stdout" 2>/dev/null || true
  if [[ "$brc" != "$rrc" ]] || ! cmp -s "$result/bash.stdout" "$result/rubash.stdout"; then
    status=DIFF
  fi
  printf '%s\t%s\t%s\t%s\n' "$name" "$status" "$brc" "$rrc" >> "$out/results.tsv"
done

total=0
pass=0
diff_count=0
first_row=yes
while IFS=$'\t' read -r name status bash_rc rubash_rc; do
  if [[ $first_row == yes ]]; then
    first_row=no
    continue
  fi
  total=$(expr "$total" + 1)
  case $status in
    PASS) pass=$(expr "$pass" + 1) ;;
    DIFF) diff_count=$(expr "$diff_count" + 1) ;;
  esac
done < "$out/results.tsv"
printf 'TOTAL=%s PASS=%s DIFF=%s\n' "$total" "$pass" "$diff_count" | tee "$out/summary.txt"
printf 'completed=yes\n' >> "$out/manifest.txt"
