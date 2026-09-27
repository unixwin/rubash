#!/usr/bin/env bash
# Perf probe 02: startup + minimal content (one function definition and call).
# Shape family: startup lazy-init for one-shot scripts with content (rubash#158).
# PERF: runs=10 timeout=20
noop() { :; }
noop
exit 0
