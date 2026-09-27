#!/usr/bin/env bash
# Perf probe 24: load (source) nvm.sh v0.40.8 into a live shell — parse +
# execute the real version-manager entry point. The #130/#155 "nvm.sh load"
# shape. NVM_DIR is pinned to a scratch dir under target/perf-corpus/.
# Run from the repo root (the harness guarantees cwd).
# PERF: runs=3 timeout=60
NVM_DIR="$PWD/target/perf-corpus/nvm-dir"
NVM_CD_FLAGS=
mkdir -p "$NVM_DIR"
. "$PWD/benchmarks/corpus/nvm.sh"
exit 0
