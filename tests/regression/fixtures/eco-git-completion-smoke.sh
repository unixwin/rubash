#!/usr/bin/env bash
# Ecosystem smoke 2/3: git-completion helper shapes. The full git-completion
# 140-case corpus is not vendored (upstream file is large and was not on
# disk at codification time); this smoke pins the helper protocol the real
# script is built on: __gitcomp-style wordlist splitting/prefix filtering,
# nested __git_ function libraries defined and called through one entry
# point, and compgen -W over subcommand names. Verified byte-identical
# against WSL GNU Bash 5.3.0.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash eco-git-completion-smoke.sh' \
#     > eco-git-completion-smoke.gnu.out 2> eco-git-completion-smoke.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
# Faithful minimal __gitcomp (git-completion.bash shape): splits the
# wordlist, optionally stops at a sentinel suffix, filters by prefix.
__gitcomp () {
  local cur_="${3-$cur}"
  case "$cur_" in
    --*=) COMPREPLY=() ;;
    *)
      local IFS=$' \t\n'
      # Real git-completion.bash form: the -P prefix is the shell-expanded
      # "${2-}" positional (usually empty). GNU compgen does NOT re-expand
      # its -P argument (a literal '${2-}' prefix stays literal there; a
      # rubash corner divergence recorded in the wt4/ci report), so this
      # fixture keeps the realistic expanded form.
      COMPREPLY=($(compgen -P "${2-}" -W "$1" -- "$cur_"))
      ;;
  esac
}
__git_subcommands () {
  echo 'add bisect branch checkout commit diff fetch grep init log merge mv pull push rebase reset revert show stash status tag'
}
__git_main () {
  local cur
  cur="${COMP_WORDS[COMP_CWORD]}"
  case "$cur" in
    st*) __gitcomp "$(__git_subcommands)" '' 'st' ;;
    *)   __gitcomp "$(__git_subcommands)" ;;
  esac
}
complete -o bashdefault -o default -o nospace -F __git_main git ec
echo "== registration"
complete -p git
echo "== complete st"
COMP_WORDS=(git st); COMP_CWORD=1; COMP_LINE='git st'; COMP_POINT=6
__git_main
printf 'compreply=[%s]\n' "${COMPREPLY[*]}"
echo "== complete re"
COMP_WORDS=(git re); COMP_CWORD=1; COMP_LINE='git re'; COMP_POINT=5
__git_main
printf 'compreply=[%s]\n' "${COMPREPLY[*]}"
echo "== helper with prefix and sentinel"
cur=''; __gitcomp 'a b abc' 'pre-'
printf 'compreply=[%s]\n' "${COMPREPLY[*]}"
echo "== nested library call through function name"
type -t __git_subcommands __git_main __gitcomp
echo done
