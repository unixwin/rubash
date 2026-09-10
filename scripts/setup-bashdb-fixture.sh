#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fixture_root="$repo_root/tests/fixtures/bashdb"
target_root="$repo_root/target/bashdb-clean"

if [[ $# -ne 1 ]]; then
  printf 'usage: %s /path/to/bashdb-checkout-or-install
' "$0" >&2
  exit 2
fi

bashdb_root=$1
if [[ ! -d "$bashdb_root" ]]; then
  printf 'bashdb checkout not found: %s
' "$bashdb_root" >&2
  exit 1
fi

mkdir -p "$target_root"
cp "$fixture_root/probe-target.sh" "$repo_root/target/bashdb-probe-target.sh"
if [[ ! -f "$target_root/getopts_long.sh" ]]; then
  cp "$fixture_root/getopts_long.sh" "$target_root/getopts_long.sh"
fi

launcher=''
for candidate in "$bashdb_root/bashdb-generated" "$bashdb_root/bashdb" "$bashdb_root/bin/bashdb"; do
  if [[ -f "$candidate" ]]; then
    launcher=$candidate
    break
  fi
done
if [[ -z "$launcher" ]]; then
  printf 'no bashdb launcher found under: %s
' "$bashdb_root" >&2
  printf 'Build/install bashdb first, then rerun this script.
' >&2
  exit 1
fi

cp "$launcher" "$target_root/bashdb-generated"
chmod +x "$target_root/bashdb-generated" 2>/dev/null || true

printf 'created %s\n' "$repo_root/target/bashdb-probe-target.sh"
printf 'created %s\n' "$target_root/getopts_long.sh"
printf 'created %s\n' "$target_root/bashdb-generated"
printf 'If the launcher embeds a different library path, invoke it with --library.\n'
