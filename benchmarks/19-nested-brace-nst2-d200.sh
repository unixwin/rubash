#!/usr/bin/env bash
# Perf probe 19: nested brace groups, depth 200, SPLIT ACROSS TWO LINES
# (opens on line 1, body+closers on line 2). The #176 "two-line" shape.
# KNOWN STATE: GNU bash parses+runs this in ~5ms; rubash (9bf2df9e) fails to
# parse ANY multi-line nested-brace layout at depth >= 2 (2-line) / >= 4
# (per-line) — this row is a canary: rubash status PARSE-FAIL until the
# skip_brace/continuation work covers multi-line nesting, then it becomes a
# timing row.
# PERF: runs=5 timeout=60
{ { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { { {
:; } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } } }
exit 0
