#!/usr/bin/env bash
# Perf probe 11: pipeline throughput — yes | head -100000.
# Shape family: builtin/external pipeline streaming (rubash#157 49.7x, #206).
# PERF: runs=5 timeout=30
yes | head -100000 >/dev/null
exit 0
