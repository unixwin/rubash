#!/usr/bin/env bash
# Perf probe 04: builtin dispatch + loop overhead — `true` x2000.
# Shape family: interpreter hot path (rubash#157/#186).
# PERF: runs=7 timeout=30
i=0
while [ "$i" -lt 2000 ]; do
  true
  i=$((i+1))
done
exit 0
