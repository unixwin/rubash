#!/usr/bin/env bash
# nounset exit-status matrix (commit 9d34920d): expr.c expr_streval raises
# FORCE_EOF for an unbound variable under set -u in every expansion
# position; shell.c:1471 run_one_command maps it to 127 under -c and 1 in
# script mode. Verified byte-identical against WSL GNU Bash 5.3.0 on
# 2026-09-27. Runs in a scratch directory: it writes sub-u.sh.
# Capture (run from an EMPTY scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash nounset-exit-matrix.sh' \
#     > nounset-exit-matrix.gnu.out 2> nounset-exit-matrix.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== plain word"
"${THIS_SH:-bash}" -uc 'echo $missing'; echo "rc=$?"
echo "== arith as argument"
"${THIS_SH:-bash}" -uc 'printf "%s\n" "$((missing + 1))"'; echo "rc=$?"
echo "== arith as assignment"
"${THIS_SH:-bash}" -uc 'x=$((missing + 1))'; echo "rc=$?"
echo "== arith command"
"${THIS_SH:-bash}" -uc '((missing + 1))'; echo "rc=$?"
echo "== colon-msg"
"${THIS_SH:-bash}" -uc 'echo ${missing?msg}'; echo "rc=$?"
echo "== script mode"
printf 'echo $m2\n' >sub-u.sh
"${THIS_SH:-bash}" -u sub-u.sh; echo "rc=$?"
echo "== bound is fine"
"${THIS_SH:-bash}" -uc 'missing=set; echo $missing'; echo "rc=$?"
echo done
