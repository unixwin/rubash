#!/usr/bin/env bash
# Perf probe 25: yes | head -100000 | while read — the rubash#206 shape.
# NOT a hang (rubash#243 verdict, 2026-09-27): the #206 fix (62c7e5fd,
# execute_external_prefix_concurrently) holds at every commit measured —
# 9bf2df9e and 7dfdaeb0 both finish rc=0 with bounded RSS (~10 MB). The
# baseline row said TIMEOUT only because timeout=20 truncated a ~23-41 s
# debug run (release: ~3.5-6 s). The read-tail loop cost (probe 13 family)
# is the amplifier; the timeout is now sized for the debug build.
# PERF: runs=3 timeout=120
yes | head -100000 | while read -r l; do :; done
exit 0
