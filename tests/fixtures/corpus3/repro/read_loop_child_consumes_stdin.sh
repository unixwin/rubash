#!/bin/sh
# rubash corpus3 lane: external command inside `while read` loop consumes the
# loop's redirected stdin -> loop ends after the first iteration.
# GNU bash 5.3.0 (WSL, script-file run): all 5 iterations on every probe.
# rubash (86818357): P2/P3 yield 1 iteration; </dev/null workaround yields 5.
set -u
f="$(dirname "$0")/read_loop_child_consumes_stdin.txt"
printf 'l1\nl2\nl3\nl4\nl5\n' > "$f"
n=0
while read -r l; do n=$((n+1)); done < "$f"
echo "P0 no-child iterations=$n"
n=0
while read -r l; do true; n=$((n+1)); done < "$f"
echo "P1 true(builtin) iterations=$n"
n=0
while read -r l; do expr "$l" : 'l.' >/dev/null; n=$((n+1)); done < "$f"
echo "P2 expr(external) iterations=$n"
n=0
while read -r l; do expr "$l" : 'l.' >/dev/null </dev/null; n=$((n+1)); done < "$f"
echo "P3 expr-devnull-stdin iterations=$n"
