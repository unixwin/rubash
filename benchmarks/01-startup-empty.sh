#!/usr/bin/env bash
# Perf probe 01: shell startup floor — init, parse nothing, exit.
# Shape family: startup cost (rubash#158); classic "shell -c exit" benchmark
# (same workload class as zsh-bench's exit / hyperfine's shell-startup runs).
# PERF: runs=10 timeout=20
exit 0
