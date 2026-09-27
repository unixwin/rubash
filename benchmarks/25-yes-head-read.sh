#!/usr/bin/env bash
# Perf probe 25: yes | head -100000 | while read — the rubash#206 shape.
# Known rubash bug: never terminates (RSS grows); expected status TIMEOUT here.
# Kept as a canary so the fix shows up as this row turning OK.
# PERF: runs=3 timeout=20
yes | head -100000 | while read -r l; do :; done
exit 0
