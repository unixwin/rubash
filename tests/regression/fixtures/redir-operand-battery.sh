#!/usr/bin/env bash
# Redirect operand word-list battery (commit 40dcee28, rubash#210): every
# file-redirect operand goes through the full word-list pipeline
# (redir.c:298 redirection_expand -> subst.c:12590 expand_words_no_vars);
# 0 or >1 fields is AMBIGUOUS_REDIRECT with the raw word
# (redir.c:187-201, 911-912). Verified byte-identical against WSL GNU Bash
# 5.3.0 on 2026-09-27. Runs in a scratch directory: it creates amb1 amb2.
# Capture (run from an EMPTY scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash redir-operand-battery.sh' \
#     > redir-operand-battery.gnu.out 2> redir-operand-battery.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
: >amb1
: >amb2
echo "== var-two-fields"
multi="amb1 amb2"; echo x >$multi; echo "rc=$?"
echo "== brace-two-words"
echo x >a{b,c}; echo "rc=$?"
echo "== brace-seq-three"
echo x >b{1..3}; echo "rc=$?"
echo "== glob-two-hits"
echo x >amb*; echo "rc=$?"
echo "== brace-empty-alt"
echo x >in{put,}file; echo "rc=$?"
echo "== glob-one-hit-opens"
echo one >amb1*; echo "rc=$? content=$(<amb1)"
echo "== brace-seq-one"
echo two >out{1..1}; echo "rc=$? content=$(<out1)"
echo "== quoted-brace-literal"
echo three >"li{teral"; echo "rc=$? content=$(<'li{teral')"
echo "== nullglob-zero"
shopt -s nullglob; echo x >zz*; echo "rc=$?"; shopt -u nullglob
echo "== posix-cat-literal-pattern"
set -o posix; read -r _l <redir1.*; echo "rc=$?"; set +o posix
echo done
