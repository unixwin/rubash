#!/usr/bin/env bash
# bash-5.3 funsub ${ cmd; } / valsub ${| cmd; } probes (commit 653b1de1,
# rubash#195). GNU owner: parse.y:4475-4516 parse_comsub /
# xparse_dolparen SX_FUNSUB (parse.y:4713-4743), executed by subst.c:6917
# function_substitute. Verified byte-identical against WSL GNU Bash 5.3.0
# on 2026-09-27.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash funsub-valsub.sh' \
#     > funsub-valsub.gnu.out 2> funsub-valsub.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== f1 basic value"
v=${ echo 4; }; echo "[$v] rc=$?"
echo "== f2 trailing newlines stripped"
v=${ printf 'a\n\n\n'; }; echo "[$v]"
echo "== f3 array element repro"
a=(1 2 3 ${ echo 4; }); echo "n=${#a[@]}: ${a[@]}"
echo "== f4 body status visible"
v=${ false; }; echo "rc=$?"; v=${ true; }; echo "rc2=$?"
echo "== f5 valsub reply"
v=${| REPLY=hello; }; echo "[$v]"
echo "== f6 valsub body stdout passes through"
v=${| echo direct; REPLY=r; }; echo "[$v]"
echo "== f7 funsub captures stdout"
v=${ echo cap; REPLY=ignored; }; echo "[$v]"
echo "== f8 funsub runs in a function context"
v=${ echo "${FUNCNAME[0]:-none}"; }; echo "[$v]"
echo "== f9 unterminated body"
"${THIS_SH:-bash}" -c 'v=${ echo 4 }'; echo "rc=$?"
echo "== f10 missing separator before brace"
"${THIS_SH:-bash}" -c 'v=${ echo 4}'; echo "rc=$?"
echo "== f11 side effect: assignment persists"
unset sv; v=${ sv=9; echo x; }; echo "sv=$sv"
echo "== f12 side effect: unset persists"
pv=1; v=${ unset pv; echo x; }; echo "pv=${pv-unset}"
echo "== f13 side effect: function def persists"
v=${ gf(){ echo inner; }; echo x; }; gf
echo "== f14 errexit cleared for body"
set -e; v=${ false; echo alive; }; echo "[$v] rc=$?"; set +e
echo "== f15 inherit_errexit keeps body failure"
shopt -s inherit_errexit; set -e
"${THIS_SH:-bash}" -ec 'v=${ false; echo alive; }; echo after'
echo "rc=$?"
shopt -u inherit_errexit; set +e
echo "== f16 unquoted result splits"
set -- ${ printf 'a b\n'; }; echo "n=$#"
echo done
