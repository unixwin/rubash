#!/usr/bin/env bash
# Perf probe 09: external command spawn loop — uname -s x300.
# Cross-OS caveat: absolute spawn cost is OS-bound (Linux fork+exec vs Windows
# CreateProcess); reported with the environment-bound flag in PERF-BASELINE.
# Shape family: fork/exec loop (the "1000x true" external variant).
# PERF: runs=7 timeout=60
i=0
while [ "$i" -lt 300 ]; do
  uname -s >/dev/null
  i=$((i+1))
done
exit 0
