#!/usr/bin/env sh
set -eu

export RUBASH_REQUIRE_BASHDB_FIXTURE=1
cargo test --test cli_tests --locked bashdb_compat -- --nocapture "$@"
