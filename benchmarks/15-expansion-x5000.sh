#!/usr/bin/env bash
# Perf probe 15: expansion-heavy loop — prefix/suffix strip + concat x5000.
# Shape family: parameter expansion hot path (subst.c parameter_brace_expand).
# PERF: runs=7 timeout=60
a="alpha:beta:gamma:delta"
i=0
while [ "$i" -lt 5000 ]; do
  b=${a#alpha}
  c=${a%%:*}
  d=${a%%gamma*}
  e="$a|$a|$a"
  i=$((i+1))
done
exit 0
