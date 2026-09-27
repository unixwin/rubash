#!/usr/bin/env bash
# Perf probe 13: read-loop throughput — generate 2000 lines, consume with read.
# Shape family: while-read throughput (read builtin + pipeline).
# PERF: runs=5 timeout=60
i=0
while [ "$i" -lt 2000 ]; do
  echo "payload line with some content $i"
  i=$((i+1))
done | while read -r l; do :; done
exit 0
