#!/usr/bin/env bash
# Extglob parse-time gate (rubash#131, commit 9bf2df9e). GNU gates extglob
# pattern operators at READ time (parse.y:5466 read_token_word consumes
# `?('/`*('/`+('/`@('/`!(' only while extended_glob is set; reset_parser
# syncs from the shopt at parse.y:3502), so without `shopt -s extglob' a
# case pattern like `-?(\[)+([a-z]))' is a syntax error near `(' with the
# offending line echoed, rc 2 — and a top-level `shopt -s extglob' flips
# the gate for later lines. The lane canary ran `bash -n' on the full
# bash_completion corpus (fails at line 1820, rc 2, both shells); this is
# the license-clean minimal form of that construct.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash extglob-parse-gate.sh' \
#     > extglob-parse-gate.gnu.out 2> extglob-parse-gate.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== gate off (default): case pattern is a syntax error"
"${THIS_SH:-bash}" -c 'opt=-abc; case $opt in -?(\[)+([a-z])) echo bundled ;; *) echo other ;; esac'
echo "rc=$?"
echo "== gate on: same pattern parses and matches"
"${THIS_SH:-bash}" -c 'shopt -s extglob; opt=-abc; case $opt in -?(\[)+([a-z])) echo bundled ;; *) echo other ;; esac'
echo "rc=$?"
echo "== bash -n form (the bash_completion line-1820 shape)"
printf 'case $o in -?([x])) echo y ;; esac\n' >mini-extglob.sh
"${THIS_SH:-bash}" -n mini-extglob.sh
echo "rc=$?"
echo "== top-level shopt flips the gate for later lines"
"${THIS_SH:-bash}" -c 'shopt -s extglob; case $o in -?([x])) echo on ;; esac; shopt -u extglob; case $o in -?([x])) echo off ;; esac'
echo "rc=$?"
echo done
