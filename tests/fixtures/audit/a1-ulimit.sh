#!/bin/bash
# Area 1 probe: ulimit option table + error paths (builtins/ulimit.def)
echo "== query forms =="
ulimit -c; echo "rc=$?"
ulimit -n; echo "rc=$?"
ulimit -u; echo "rc=$?"
ulimit -f; echo "rc=$?"
ulimit -t; echo "rc=$?"
echo "== set soft limit =="
ulimit -n 2048; echo "rc=$?"; ulimit -n
ulimit -c 100; echo "rc=$?"; ulimit -c
ulimit -u 4096; echo "rc=$?"
ulimit -f unlimited; echo "rc=$?"
echo "== hard/soft modifiers =="
ulimit -S -n 1024; echo "rc=$?"
ulimit -H -n 2>/dev/null; echo "rc=$?"
echo "== invalid letter / invalid number =="
ulimit -z; echo "rc=$?"
ulimit -n abc; echo "rc=$?"
ulimit -c -5; echo "rc=$?"
echo "== unknown arg =="
ulimit +1999; echo "rc=$?"
echo "== ulimit -a format (labels+flags only) =="
ulimit -a | sed 's/[0-9][0-9]*$//; s/unlimited$//'
