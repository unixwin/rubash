#!/usr/bin/env bash
# Perf probe 05: arithmetic evaluation — (( i += 1 )) x5000.
# Shape family: arithmetic hot loop (rubash#156: 30.7x).
# PERF: runs=7 timeout=30
i=0
while (( i < 5000 )); do
  (( i += 1 ))
done
exit 0
