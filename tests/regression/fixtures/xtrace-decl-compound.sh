#!/usr/bin/env bash
# xtrace declaration compound assignments (commit f3854d6a, probe-xtrace5.sh).
# Verified byte-identical against WSL GNU Bash 5.3.0 on 2026-09-27.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash xtrace-decl-compound.sh' \
#     > xtrace-decl-compound.gnu.out 2> xtrace-decl-compound.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
set -x
x=([a]=1)
z=(1 2)
v=([k]="s p")
echo hi
w=([b]=2) echo hi
w=() echo hi
a=1 declare -A m=([q]=r)
declare -A mm=([q]=r)
declare -a n=(1 2 'x y')
declare -a d=($'a\tb')
declare -A s+=( [k2]=v2 )
echo "${s[k2]}"
# Function-invocation head line (issue #247): execute_cmd.c:4649 traces the
# command head before function dispatch, so each call prints `+ f' ahead of
# the body's own traces (recursion nests one head line per call).
f() { echo body; }
f
f one "two three"
g() { f; }
g
set +x
echo done
