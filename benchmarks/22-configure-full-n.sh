#!/usr/bin/env bash
# Perf probe 22: parse-only ($SH -n) of GNU bash's own configure script in
# full (24753 lines, LF copy under target/perf-corpus/, harness-prepared).
# NOTE: rubash currently aborts with a syntax error at line 5345
# (multi-line as_fn_error message string) — its number covers a PARTIAL
# parse (~21.6% of the file) and is flagged non-OK in the table.
# Shape family: #130/#155/#178 parse-throughput family at full scale.
# PERF: file=target/perf-corpus/configure args="-n" runs=3 timeout=120
exit 0
