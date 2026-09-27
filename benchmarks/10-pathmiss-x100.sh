#!/usr/bin/env bash
# Perf probe 10: PATH search miss — command -v <unique-missing> x100.
# Unique names defeat any lookup cache; measures the full PATH scan cost.
# Shape family: PATH miss storm (rubash#159: 14.2ms per miss).
# PERF: runs=7 timeout=60
i=0
while [ "$i" -lt 100 ]; do
  command -v "rubash-perf-missing-$i"
  i=$((i+1))
done
exit 0
