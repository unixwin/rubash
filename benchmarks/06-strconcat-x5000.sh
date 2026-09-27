#!/usr/bin/env bash
# Perf probe 06: string append assignment — s+=x x5000.
# Shape family: assignment hot path (rubash#157: s+=x 26.6x).
# PERF: runs=7 timeout=30
s=
i=0
while [ "$i" -lt 5000 ]; do
  s+=x
  i=$((i+1))
done
exit 0
