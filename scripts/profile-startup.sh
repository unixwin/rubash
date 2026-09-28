#!/usr/bin/env bash
# Attribution profiling for rubash/niubash (the missing half of the perf
# infrastructure: run-perf-suite.sh measures WHICH scenario is slow, this
# records WHERE inside the binary the time goes).
#
# Usage:
#   bash scripts/profile-startup.sh rubash 'echo ok'        # engine startup
#   bash scripts/profile-startup.sh niu -C 'echo ok'        # host + rc + OMB load
#   bash scripts/profile-startup.sh niu --script probe.sh   # custom scenario file
#
# Output: target/perf-profiles/<label>-<timestamp>.json (Firefox Profiler
# format). Open at https://profiler.firefox.com (Import) for the flame graph,
# or `samply load <file>` to relaunch the local viewer.
#
# Requires: cargo install samply --locked
set -euo pipefail

kind="${1:?usage: profile-startup.sh rubash|niu <args...>}"
shift
label="$(date +%Y%m%d-%H%M%S)"
out_dir="target/perf-profiles"
mkdir -p "$out_dir"

case "$kind" in
  rubash)
    exe="target/release/rubash.exe"
    ;;
  niu)
    exe="$HOME/AppData/Local/Programs/Winuxsh/niu.exe"
    ;;
  *)
    echo "unknown kind: $kind (rubash|niu)" >&2; exit 2;;
esac

if [ "$1" = "--script" ]; then
  scenario=("$(cygpath -w "$2")")
  name="$(basename "$2" .sh)"
else
  scenario=("$@")
  name="cmd"
fi

out="$out_dir/${kind}-${name}-${label}.json"
echo "[profile] $exe ${scenario[*]}"
echo "[profile] saving to $out"
samply record --save-only -o "$out" -- "$exe" "${scenario[@]}"
echo "[profile] done — open $out at https://profiler.firefox.com (import)"
