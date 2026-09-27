#!/usr/bin/env bash
# Perf probe 21: parse-only ($SH -n) of the first 1374 lines of GNU bash's
# own configure script (autoconf-generated; LF copy prepared by the harness
# under target/perf-corpus/; cut at a `done`+blank top-level boundary so both
# shells parse the same complete statements). Real-world parser throughput.
# Shape family: #130/#155/#178 parse-throughput family.
# PERF: file=target/perf-corpus/configure-head1374 args="-n" runs=3 timeout=60
exit 0
