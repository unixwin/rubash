#!/bin/bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# Run all failing Bash official tests

TESTS_DIR="third_party/bash/tests"
RUBASH="target/debug/rubash.exe"
OUTPUT_DIR="target/test-analysis-output"

mkdir -p "$OUTPUT_DIR"

TESTS="arith array braces builtins comsub-posix comsub2 cond mapfile posixexp2 quotearray alias quote getopts printf trap complete glob histexp posix2 posixpipe rsh"

echo "=== Running failing Bash official tests ==="

for test in $TESTS; do
  echo "--- Testing: $test ---"
  
  timeout 30 $RUBASH "$TESTS_DIR/${test}.tests" > "$OUTPUT_DIR/${test}.rubash.out" 2> "$OUTPUT_DIR/${test}.rubash.err"
  rubash_rc=$?
  
  timeout 30 bash "$TESTS_DIR/${test}.tests" > "$OUTPUT_DIR/${test}.bash.out" 2> "$OUTPUT_DIR/${test}.bash.err"
  bash_rc=$?
  
  echo "  Rubash RC: $rubash_rc, Bash RC: $bash_rc"
  
  if [ $rubash_rc -ne $bash_rc ]; then
    echo "  *** STATUS MISMATCH ***"
  fi
  
  if [ -s "$OUTPUT_DIR/${test}.rubash.err" ]; then
    echo "  Rubash stderr (first 3 lines):"
    head -3 "$OUTPUT_DIR/${test}.rubash.err" | sed 's/^/    /'
  fi
  
  echo ""
done

echo "=== Done ==="