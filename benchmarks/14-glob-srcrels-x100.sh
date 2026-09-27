#!/usr/bin/env bash
# Perf probe 14: glob-heavy loop — set -- src/*/*.rs src/*/*/*.rs x100.
# ~290 committed files per iteration; parse-only workload, no external cmds.
# Env caveat: GNU side globs through WSL drvfs (/mnt/d), which inflates its
# side; the ratio is therefore conservative (understated).
# Shape family: glob loop (rubash#157: glob 39.3x).
# PERF: runs=5 timeout=120
i=0
while [ "$i" -lt 100 ]; do
  set -- src/*/*.rs src/*/*/*.rs
  i=$((i+1))
done
exit 0
