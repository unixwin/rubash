#!/usr/bin/env bash
# Perf probe 12: builtin-to-builtin pipeline — echo | while read x2000.
# Two-process pipeline of builtins; the #157 "builtin pipeline 49.7x" shape
# and the #186 per-command ast_exec preamble amplifier.
# PERF: runs=5 timeout=90
i=0
while [ "$i" -lt 2000 ]; do
  echo hi | while read -r l; do :; done
  i=$((i+1))
done
exit 0
