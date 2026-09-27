#!/usr/bin/env bash
# Perf probe 07: no-op function call x5000 — call frame + dispatch cost.
# Shape family: function call overhead (rubash#157: 27.8x / #186 preamble).
# PERF: runs=7 timeout=60
f() { :; }
i=0
while [ "$i" -lt 5000 ]; do
  f
  i=$((i+1))
done
exit 0
