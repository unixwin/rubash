#!/usr/bin/env bash
# Function-body strictness battery (commit 40dcee28, rubash#213): a stray
# `)' or `;;' inside a function body is a parse error with the offending
# line echoed verbatim and exit 2 (parse.y:6724 -> 6833 -> 6861/6867).
# Abort shapes run in inner -c shells so the battery itself continues;
# valid bodies run inline. Verified byte-identical against WSL GNU Bash
# 5.3.0 on 2026-09-27.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash fnbody-strict-battery.sh' \
#     > fnbody-strict-battery.gnu.out 2> fnbody-strict-battery.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== stray: brace body"
"${THIS_SH:-bash}" -c 'f() { echo ); }'; echo "rc=$?"
echo "== stray: multiline body"
"${THIS_SH:-bash}" -c 'f() {
  echo )
}'; echo "rc=$?"
echo "== stray: keyword form"
"${THIS_SH:-bash}" -c 'function f { echo ); }'; echo "rc=$?"
echo "== stray: subshell body"
"${THIS_SH:-bash}" -c 'f() ( echo ); )'; echo "rc=$?"
echo "== stray: double-semi in body"
"${THIS_SH:-bash}" -c 'f() { echo ;; }'; echo "rc=$?"
echo "== stray: top-level double-semi"
"${THIS_SH:-bash}" -c 'echo ;;'; echo "rc=$?"
echo "== valid: case body"
vc() { case x in x) echo case-ok;; esac; }; vc
echo "== valid: if body"
vi() { if true; then echo if-ok; fi; }; vi
echo "== valid: for body"
vf() { for i in 1 2; do echo "for-$i"; done; }; vf
echo "== valid: while body"
vw() { while read -r l; do echo "while-$l"; done <<< line1; }; vw
echo "== valid: subshell body"
vs() ( echo sub-ok ); vs
echo "== valid: nested braces"
vb() { { echo brace-ok; } }; vb
echo "== valid: heredoc body"
vh() { while read -r hl; do echo "h:$hl"; done <<'EOF'
heredoc-ok
EOF
}; vh
echo "== valid: arith body"
va() { (( 6*7 == 42 )) && echo arith-ok; }; va
echo "== valid: quoted parens"
vq() { echo "quoted-(parens)-ok"; }; vq
echo "== valid: comment in body"
vc2() { # a comment
  echo comment-ok; }; vc2
echo "== valid: command-sub subshell body"
vcs() ( echo "csub-$(echo ok)" ); vcs
echo "== valid: nested def"
vn() { vni() { echo nested-ok; }; vni; }; vn
echo "== valid: keyword form"
function vk { echo keyword-form-ok; }; vk
echo "== valid: array body"
var() { local a=(x y); echo "array-${a[1]}"; }; var
echo done
