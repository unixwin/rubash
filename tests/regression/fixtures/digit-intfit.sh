#!/usr/bin/env bash
# Digit redirect-prefix int-fit gate (commit 22abdef9, rubash#197):
# parse.y:5725-5738 got_token treats all-digit words before `<'/`>' as an
# fd prefix only when the value fits in int; out-of-range digits become a
# plain WORD (command name). Verified byte-identical against WSL GNU Bash
# 5.3.0 on 2026-09-27 (d3's 2147483647 case is an RLIMIT_NOFILE artifact
# and is excluded). Runs in an EMPTY scratch directory: it creates d10.txt
# and normal-fd.txt.
# Capture (run from an EMPTY scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash digit-intfit.sh' \
#     > digit-intfit.gnu.out 2> digit-intfit.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== d1 i64 overflow inside comsub"
"${THIS_SH:-bash}" -c 'echo "[$(1111111111111111112222</dev/null)]"'; echo "rc=$?"
echo "== d2 i32 boundary+1 adjacency"
"${THIS_SH:-bash}" -c '2147483648</dev/null'; echo "rc=$?"
echo "== d4 normal fd redirect still works"
exec 3>&1; echo fd3-ok >&3; exec 3>&-; echo "rc3=$?"
echo ten-ok 10>d10.txt; echo "rc10=$?"
printf 'body\n' >normal-fd.txt
{ read -r fd4line <&4; echo "fd4=$fd4line"; } 4<normal-fd.txt; echo "rc4=$?"
exec 4<&-
echo done
