#!/usr/bin/env bash
# Perf probe 08: command substitution of a builtin — x=$(true) x1000.
# Measures subshell/fork-equivalent spawn cost without depending on any
# external binary (GNU forks; rubash spawns whatever its comsub backend is).
# Shape family: subshell + comsub execution (subst.c command_substitute).
# PERF: runs=7 timeout=60
i=0
while [ "$i" -lt 1000 ]; do
  x=$(true)
  i=$((i+1))
done
exit 0
