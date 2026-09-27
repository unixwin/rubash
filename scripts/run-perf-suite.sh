#!/usr/bin/env bash
# run-perf-suite.sh — repeatable rubash performance suite.
#
# Runs every numbered probe under benchmarks/ against BOTH:
#   - rubash:  $ROOT/target/debug/rubash.exe (override with RUBASH=...)
#   - GNU:     WSL GNU Bash 5.3.0 (/usr/local/bin/bash, owner-compiled)
# and emits a Markdown ratio table.
#
# Usage (from the repo root, any bash >= 4; Git Bash recommended on Windows):
#   bash scripts/run-perf-suite.sh                 # full suite
#   bash scripts/run-perf-suite.sh --probe 16      # only probes matching "16"
#   bash scripts/run-perf-suite.sh --runs 3        # cap repetitions at 3
#
# Methodology (portable timing harness; hyperfine is NOT used — it is not
# installed on the measurement host and per-run `wsl.exe` startup overhead
# would corrupt the GNU side. If hyperfine is installed someday, keep the
# methodology parity rule: either side must be measured the same way):
#   per probe: 1 validation run (rc gate) + N timed runs, wall clock via
#   date +%s%N around the child, median reported. timeout(1) bounds each run.
#   GNU side is timed INSIDE WSL by re-invoking this script with --inner, so
#   wsl.exe round-trip cost never lands in the numbers.
#
# Probe metadata (in-file comments, all optional):
#   # PERF: runs=5 timeout=60 args="-n" file=path/or/sibling.sh
#     runs    repetitions (default 7)
#     timeout per-run timeout seconds (default 60)
#     args    extra shell args before the target file, space-separated tokens
#     file    target file if not the probe itself; resolved against the
#             benchmarks/ dir first, then the repo root
#
# Statuses: OK | RCn (ran, non-zero rc, still timed) | TIMEOUT (killed).
# Output: target/perf-results/{rubash.tsv,gnu.tsv,PERF-TABLE.md}

set -u

# ---- clean environment (AGENTS.md: keep the measurement honest) ----------
unset BASH_ENV 2>/dev/null || true
unset WINUXSH_ROOT 2>/dev/null || true
unset -f rm rmdir unlink 2>/dev/null || true

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BENCH="$ROOT/benchmarks"
RESULTS="$ROOT/target/perf-results"
GNU_BASH=/usr/local/bin/bash   # WSL owner-compiled 5.3.0 (AGENTS.md oracle)

FILTER=""
RUNS_CAP=""
INNER=0
INNER_OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --inner) INNER=1 ;;
    --out) INNER_OUT="$2"; shift ;;
    --probe) FILTER="$2"; shift ;;
    --runs) RUNS_CAP="$2"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# ---- helpers --------------------------------------------------------------

meta_field() { # $1=file $2=key -> value (empty if absent; quotes stripped)
  # Pure-bash parsing (no grep/tr/cut children): a pipe-based version lost
  # values intermittently on the WSL side, silently turning args="-n" probes
  # into executions.
  local line tok k v
  while IFS= read -r line; do
    case "$line" in
      "# PERF:"*) ;;
      *) continue ;;
    esac
    for tok in $line; do
      k="${tok%%=*}"
      if [ "$k" = "$2" ]; then
        v="${tok#*=}"
        v="${v#\"}"; v="${v%\"}"
        printf '%s\n' "$v"   # printf, not echo: values like "-n" are eaten by echo
        return 0
      fi
    done
  done < "$1"
  return 0
}

resolve_target() { # $1=repo-root $2=bench-dir $3=file-value $4=probe-path
  local root="$1" bench="$2" fval="$3" probe="$4"
  if [ -z "$fval" ]; then
    echo "$probe"
  elif [ -f "$bench/$fval" ]; then
    echo "$bench/$fval"
  elif [ -f "$root/$fval" ]; then
    echo "$root/$fval"
  else
    echo "$probe"   # let the run report the failure
  fi
}

median_of() { # numbers... -> median
  local sorted
  sorted=$(printf '%s\n' "$@" | sort -n)
  local n
  n=$(printf '%s\n' $sorted | wc -l)
  local idx=$(( (n + 1) / 2 ))
  printf '%s\n' $sorted | sed -n "${idx}p"
}

run_probe() { # $1=shell $2=probe-path $3=root -> appends TSV line to $TSV
  local sh="$1" probe="$2" root="$3"
  local name runs timeout_s args fval tgt rc t0 t1 ms status
  name="$(basename "$probe" .sh)"
  runs="$(meta_field "$probe" runs)";    [ -n "$runs" ] || runs=7
  timeout_s="$(meta_field "$probe" timeout)"; [ -n "$timeout_s" ] || timeout_s=60
  args="$(meta_field "$probe" args)"
  fval="$(meta_field "$probe" file)"
  if [ -n "$RUNS_CAP" ] && [ "$runs" -gt "$RUNS_CAP" ]; then runs="$RUNS_CAP"; fi
  tgt="$(resolve_target "$root" "$BENCH" "$fval" "$probe")"

  local cmd=("$sh")
  if [ -n "$args" ]; then
    local a
    for a in $args; do cmd+=("$a"); done   # args tokens never contain spaces
  fi
  if [ "$(meta_field "$probe" nofile)" != "1" ]; then
    cmd+=("$tgt")
  fi

  # validation run (also serves as warmup); -k: SIGKILL follow-up because
  # interactive shells ignore SIGTERM (job control)
  timeout -k 5 "$timeout_s" "${cmd[@]}" >/dev/null 2>&1 </dev/null
  rc=$?
  if [ "$rc" -eq 124 ]; then
    printf '%s\tTIMEOUT\t-\t-\t-\t%s\t124\n' "$name" "$runs" >> "$TSV"
    return
  fi
  if [ "$rc" -eq 0 ]; then status=OK; else status="RC$rc"; fi

  local times=() i
  for i in $(seq 1 "$runs"); do
    t0=$(date +%s%N)
    timeout -k 5 "$timeout_s" "${cmd[@]}" >/dev/null 2>&1 </dev/null
    rc=$?
    t1=$(date +%s%N)
    if [ "$rc" -eq 124 ]; then
      printf '%s\tTIMEOUT\t-\t-\t-\t%s\t124\n' "$name" "$runs" >> "$TSV"
      return
    fi
    times+=($(( (t1 - t0) / 1000000 )))
  done
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$name" "$status" "$(median_of "${times[@]}")" \
    "$(printf '%s\n' "${times[@]}" | sort -n | head -1)" \
    "$(printf '%s\n' "${times[@]}" | sort -n | tail -1)" \
    "$runs" "$rc" >> "$TSV"
}

enumerate_probes() {
  local p
  for p in "$BENCH"/[0-9][0-9]-*.sh; do
    [ -f "$p" ] || continue
    if [ -n "$FILTER" ]; then
      case "$(basename "$p")" in *"$FILTER"*) ;; *) continue ;; esac
    fi
    echo "$p"
  done
}

prepare_corpus() { # idempotent; LF copies under target/perf-corpus/
  mkdir -p "$ROOT/target/perf-corpus/nvm-dir" "$RESULTS"
  tr -d '\r' < "$ROOT/third_party/bash/configure" > "$ROOT/target/perf-corpus/configure"
  head -1374 "$ROOT/target/perf-corpus/configure" > "$ROOT/target/perf-corpus/configure-head1374"
}

to_wsl_path() { # $1 = repo root (msys /d/... form) -> /mnt/<drive>/... form
  local p="$1"
  case "$p" in
    /mnt/*) echo "$p" ;;                              # already WSL-form
    /?/*)  echo "$p" | sed -e 's|^/\([a-zA-Z]\)/|/mnt/\1/|' ;;
    *)     echo "" ;;
  esac
}

# ---- inner mode: run inside WSL, GNU side only ----------------------------

if [ "$INNER" -eq 1 ]; then
  BENCH="$(cd "$ROOT/benchmarks" && pwd)"
  export HOME="/tmp/rubash-perf-home.$$"
  mkdir -p "$HOME"
  # Linux-only PATH: WSL interop appends Windows dirs, which makes every PATH
  # miss scan slow 9p/drvfs mounts and poisons spawn/lookup timings.
  export PATH=/usr/local/bin:/usr/bin:/bin
  TSV="${INNER_OUT:-$ROOT/target/perf-results/gnu.tsv}"
  if [ ! -s "$TSV" ]; then
    echo "# gnu $("$GNU_BASH" --version | head -1)" > "$TSV"
    echo "# uname $(uname -srm) $(uname -r)" >> "$TSV"
  fi
  # resumable: skip probes that already have a result line
  for probe in $(enumerate_probes); do
    name="$(basename "$probe" .sh)"
    if awk -F'\t' -v n="$name" '$1 == n { found = 1 } END { exit !found }' "$TSV"; then
      continue
    fi
    run_probe "$GNU_BASH" "$probe" "$ROOT"
  done
  exit 0
fi

# ---- outer mode -----------------------------------------------------------

RUBASH="${RUBASH:-$ROOT/target/debug/rubash.exe}"
if [ ! -x "$RUBASH" ]; then
  echo "FATAL: rubash binary not found/executable: $RUBASH" >&2
  echo "       build first (cargo build) or set RUBASH=<path>" >&2
  exit 1
fi
prepare_corpus

mkdir -p "$RESULTS"
TSV="$RESULTS/rubash.tsv"
if [ ! -s "$TSV" ]; then
  {
    echo "# rubash $($RUBASH --version 2>/dev/null | head -1)"
    echo "# exe $RUBASH ($(stat -c %s "$RUBASH" 2>/dev/null || wc -c < "$RUBASH") bytes)"
    echo "# commit $(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "# uname $(uname -srm 2>/dev/null || echo unknown)"
    echo "# date $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$TSV"
fi
# resumable: skip probes that already have a result line
skip_present() { # $1=tsv $2=name -> 0 if present
  awk -F'\t' -v n="$2" '$1 == n { found = 1 } END { exit !found }' "$1" 2>/dev/null
}

for probe in $(enumerate_probes); do
  name="$(basename "$probe" .sh)"
  skip_present "$TSV" "$name" && continue
  run_probe "$RUBASH" "$probe" "$ROOT"
done

# GNU side: one wsl.exe round-trip; all timing happens inside WSL.
WSLROOT="$(to_wsl_path "$ROOT")"
if [ -z "$WSLROOT" ]; then
  echo "FATAL: could not map repo root ($ROOT) to a /mnt/<drive>/... path" >&2
  exit 1
fi
wsl_args=(bash "$WSLROOT/scripts/run-perf-suite.sh" --inner --out "$WSLROOT/target/perf-results/gnu.tsv")
if [ -n "$FILTER" ]; then wsl_args+=(--probe "$FILTER"); fi
if [ -n "$RUNS_CAP" ]; then wsl_args+=(--runs "$RUNS_CAP"); fi
# Long no-output wsl.exe invocations can drop their session (observed rc=9);
# the inner pass is resumable, so retry until every rubash row has a GNU row.
attempt=0
while [ "$attempt" -lt 3 ]; do
  MSYS_NO_PATHCONV=1 wsl.exe "${wsl_args[@]}" || echo "WARN: inner GNU pass rc=$? (attempt $attempt)" >&2
  missing=0
  for probe in $(enumerate_probes); do
    name="$(basename "$probe" .sh)"
    skip_present "$RESULTS/gnu.tsv" "$name" || missing=1
  done
  [ "$missing" -eq 0 ] && break
  attempt=$((attempt + 1))
  echo "WARN: GNU results incomplete, retrying inner pass (attempt $attempt)" >&2
done

# ---- merge into a Markdown table ------------------------------------------

table="$RESULTS/PERF-TABLE.md"
rub_line="$($RUBASH --version 2>/dev/null | head -1)"
gnu_line="$(sed -n 's/^# gnu //p' "$RESULTS/gnu.tsv" 2>/dev/null)"
commit_line="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
gen_line="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  echo "# Perf suite results"
  echo
  echo "- rubash: \`$rub_line\`"
  echo "- GNU:    \`$gnu_line\`"
  echo "- commit: \`$commit_line\`"
  echo "- generated: $gen_line"
  echo
  echo "| probe | rubash ms | GNU ms | ratio | status |"
  echo "|---|---:|---:|---:|---|"
} > "$table"

join_field() { # $1=tsv $2=probe $3=field-index
  awk -F'\t' -v p="$2" -v i="$3" '$1 == p { print $i; exit }' "$1" 2>/dev/null
}

FILTER_BACKUP="$FILTER"
FILTER=""   # the summary table always covers the whole suite
for probe in $(enumerate_probes); do
  name="$(basename "$probe" .sh)"
  rst="$(join_field "$RESULTS/rubash.tsv" "$name" 2)"
  gst="$(join_field "$RESULTS/gnu.tsv" "$name" 2)"
  rmed="$(join_field "$RESULTS/rubash.tsv" "$name" 3)"
  gmed="$(join_field "$RESULTS/gnu.tsv" "$name" 3)"
  ratio="-"
  if [ "$rmed" != "-" ] && [ "$gmed" != "-" ] && [ -n "$rmed" ] && [ -n "$gmed" ] \
     && [ "$gmed" -gt 0 ] 2>/dev/null; then
    ratio=$(awk -v r="$rmed" -v g="$gmed" 'BEGIN { printf "%.1fx", r / g }')
    if [ "$rst" = OK ] && [ "$gst" = OK ]; then
      over=$(awk -v r="$rmed" -v g="$gmed" 'BEGIN { print (r / g >= 10) ? 1 : 0 }')
      [ "$over" = 1 ] && ratio="**$ratio**"
    fi
  fi
  [ -n "$rst" ] || rst="MISSING"
  [ -n "$gst" ] || gst="MISSING"
  status="$rst"
  [ "$gst" != "$rst" ] && status="$rst / gnu:$gst"
  echo "| $name | ${rmed:--} | ${gmed:--} | $ratio | $status |" >> "$table"
done
FILTER="$FILTER_BACKUP"

cat "$table"
echo
echo "Artifacts: $RESULTS/{rubash.tsv,gnu.tsv,PERF-TABLE.md}"
