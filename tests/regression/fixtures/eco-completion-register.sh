#!/usr/bin/env bash
# Ecosystem smoke 1/3: bash-completion registration protocol. The full
# bash-completion 452-case corpus is not vendored (too large, not on disk
# at codification time); this smoke pins the API surface every
# bash-completion loader exercises: complete -F/-o registration,
# complete -p re-listing, direct completion-function invocation with
# COMP_WORDS/COMP_CWORD/COMP_LINE bound, and compgen -W/-f/-A results.
# Verified byte-identical against WSL GNU Bash 5.3.0.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash eco-completion-register.sh' \
#     > eco-completion-register.gnu.out 2> eco-completion-register.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
_comp_myprog() {
  local cur prev
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"
  case "$prev" in
    --mode) COMPREPLY=($(compgen -W "fast slow" -- "$cur")) ;;
    *) COMPREPLY=($(compgen -W "--mode --help start stop" -- "$cur")) ;;
  esac
}
complete -F _comp_myprog -o default myprog
echo "== registration"
complete -p myprog
echo "== invoke for --m"
COMP_WORDS=(myprog --m); COMP_CWORD=1; COMP_LINE='myprog --m'; COMP_POINT=9
_comp_myprog
printf 'compreply=[%s]\n' "${COMPREPLY[*]}"
echo "== invoke for --mode fa"
COMP_WORDS=(myprog --mode fa); COMP_CWORD=2; COMP_LINE='myprog --mode fa'; COMP_POINT=15
_comp_myprog
printf 'compreply=[%s]\n' "${COMPREPLY[*]}"
echo "== compgen wordlist"
compgen -W "alpha beta gamma" -- 'a'
echo "== compgen files"
: >reg-a.one
: >reg-b.two
compgen -f -- 'reg-'
echo "== compgen -A directory"
mkdir -p reg-dir
compgen -A directory -- 'reg-'
echo "== load leaves stderr empty"
type -t _comp_myprog
echo done
