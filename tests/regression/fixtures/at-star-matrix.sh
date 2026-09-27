#!/usr/bin/env bash
# Unquoted $@/$* splitting, 14-case matrix (commit 7f0c67a4,
# probe-at-matrix.sh). GNU model: subst.c:2957 string_list_dollar_at joins
# the parameters into ONE string with the first IFS char, then splits that
# string with the standard IFS rules. Verified byte-identical against WSL
# GNU Bash 5.3.0 on 2026-09-27.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash at-star-matrix.sh' \
#     > at-star-matrix.gnu.out 2> at-star-matrix.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
run() { echo "== $1"; shift; "${THIS_SH:-bash}" -c "$*"; }
run "at-default"          'set -- a "b c" d; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-default-empty"    'set -- a "" d; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-colon"            'IFS=:; set -- a "b c" d; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-colon-empty"      'IFS=:; set -- a "" d; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-colon-sepinside"  'IFS=:; set -- "a:b" c; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-mixed-ifs"        'IFS=" :"; set -- "a b" "" c; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-null-ifs"         'IFS=""; set -- a "b c" d; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "star-default"        'set -- a "b c" d; set -- $*; echo "n=$#"; printf "[%s]" "$@"; echo'
run "star-colon-empty"    'IFS=:; set -- a "" d; set -- $*; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-newexp-modified"  'set -- . x; set -- ${@%%[!/]*}; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-newexp-nonempty"  'set -- ax bx; set -- ${@%%x}; echo "n=$#"; printf "[%s]" "$@"; echo'
run "at-arg-with-glob"    'set -- "*.c" b; set -- $@; echo "n=$#"; printf "[%s]" "$@"; echo'
run "for-at-default"      'set -- a "" b; for w in $@; do echo "<$w>"; done'
run "cmd-arg-at"          'set -- a "" b; printf "(%s)" $@; echo'
