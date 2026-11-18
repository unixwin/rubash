#!/usr/bin/env bash
# rubash#2907 chain driver (in-repo pin fixture; authored for the
# ecosuite, first run as eco-2907 in ~/workspace/eco-2907/). The REAL
# git-completion.bash / git-prompt.sh are GPL and are NOT vendored:
# the harness (tests/issue2907_git_contrib_chain.rs) discovers them on
# the host and passes their paths in via GITCOMP / GITPROMPT.
# Run UNDER the candidate shell (GNU bash / rubash).
# Env in: REPOS (test repos root), GITCOMP (git-completion.bash), GITPROMPT (git-prompt.sh).
# Prints machine-comparable sections; every step asserts intermediate state.
# NOTE: no `set -u` here on purpose: the real git-prompt.sh references
# GIT_PS1_SHOWCONFLICTSTATE unguarded (line 514) and fails identically
# under GNU bash with `set -u`; interactive shells run `set +u`.
# git 2.43.0 has NO `_git` symbol (only per-subcommand `_git_add` ...);
# the registered live entry point is `__git_wrap__git_main`, wrapping
# `__git_main` via `__git_func_wrap`. Both are driven below.

say() { printf '%s\n' "$*"; }

# ---------- Chain A: real git-completion.bash, live _git attempts ----------
say '== A0 source git-completion.bash'
# shellcheck disable=SC1090
. "$GITCOMP"
say "A0 source rc=$?"
say "A0 type __git_wrap__git_main: $(type -t __git_wrap__git_main 2>&1)"
say "A0 type __git_main: $(type -t __git_main 2>&1)"
say "A0 type _git: $(type -t _git 2>&1)"
say "A0 type _get_comp_words_by_ref: $(type -t _get_comp_words_by_ref 2>&1)"
say "A0 complete -p git: $(complete -p git 2>&1)"
say "A0 complete -p gitk: $(complete -p gitk 2>&1)"

# run_case <id> <repo-subdir> <words...>
# Sets COMP_WORDS/COMP_CWORD/COMP_LINE/COMP_POINT, calls _git, dumps COMPREPLY one per line.
run_case() {
  local id="$1"; shift
  local sub="$1"; shift
  cd "$REPOS/$sub" || { say "CASE $id cd FAILED"; return 1; }
  COMP_WORDS=("$@")
  COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
  COMP_LINE="${COMP_WORDS[*]}"
  COMP_POINT=${#COMP_LINE}
  COMPREPLY=()
  __git_wrap__git_main
  local rc=$?
  say "CASE $id rc=$rc cword=$COMP_CWORD words=[${COMP_WORDS[*]}] nreply=${#COMPREPLY[@]}"
  local i
  for i in "${COMPREPLY[@]}"; do printf 'REPLY|%s\n' "$i"; done
  say "ENDCASE $id"
}

# Repo 'main' has: branches main, feature/login; tags v1.0, v1.1;
# files README.md, src/main.rs, docs/guide.md; remote origin.
run_case A1-subcmd   main git ch
run_case A2-branch   main git checkout fe
run_case A3-tag      main git checkout v
run_case A4-logopt   main git log --o
run_case A5-subcmd2  main git sta
run_case A6-file     main git add READ
run_case A7-filedir  main git add src/
run_case A8-remote   main git remote show o
run_case A9-branchopt main git branch --l
run_case A10-commitopt main git commit --a
run_case A11-diffopt main git diff --
run_case A12-stash   main git stash l
run_case A13-empty   main git checkout ''
run_case A14-subcmd3 main git mer
# A15: drive the inner dispatcher __git_main directly (same words as A2).
cd "$REPOS/main" || say 'CASE A15 cd FAILED'
COMP_WORDS=(git checkout fe); COMP_CWORD=2; COMP_LINE='git checkout fe'; COMP_POINT=15
COMPREPLY=()
__git_main
say "CASE A15-direct-__git_main rc=$? cword=$COMP_CWORD words=[${COMP_WORDS[*]}] nreply=${#COMPREPLY[@]}"
for i in "${COMPREPLY[@]}"; do printf 'REPLY|%s\n' "$i"; done
say 'ENDCASE A15-direct-__git_main'

# ---------- Chain B: real git-prompt.sh, __git_ps1 per repo state ----------
say '== B0 source git-prompt.sh'
# shellcheck disable=SC1090
. "$GITPROMPT"
say "B0 source rc=$?"
say "B0 type __git_ps1: $(type -t __git_ps1 2>&1)"

# prompt_case <id> <repo-subdir> <env-assignments...>
prompt_case() {
  local id="$1"; shift
  local sub="$1"; shift
  cd "$REPOS/$sub" || { say "PROMPT $id cd FAILED"; return 1; }
  GIT_PS1_SHOWDIRTYSTATE= GIT_PS1_SHOWUNTRACKEDFILES= GIT_PS1_SHOWSTASHSTATE= GIT_PS1_SHOWUPSTREAM=
  # shellcheck disable=SC2034
  local a
  for a in "$@"; do eval "$a"; done
  local out
  out=$(__git_ps1 '(%s)')
  say "PROMPT $id fmt rc=$? out=[$out]"
  say "ENDPROMPT $id"
}

prompt_case B1-clean        clean
prompt_case B2-dirty       dirty  GIT_PS1_SHOWDIRTYSTATE=1
prompt_case B3-staged      staged GIT_PS1_SHOWDIRTYSTATE=1
prompt_case B4-untracked   untracked GIT_PS1_SHOWUNTRACKEDFILES=1
prompt_case B5-all         all    GIT_PS1_SHOWDIRTYSTATE=1 GIT_PS1_SHOWUNTRACKEDFILES=1 GIT_PS1_SHOWSTASHSTATE=1
prompt_case B6-detached    detached
prompt_case B7-ahead       ahead  GIT_PS1_SHOWUPSTREAM=auto
prompt_case B8-dirty-plain dirty
prompt_case B9-untracked-plain untracked

# B10: PS1-mutation form (no format arg) in dirty repo.
cd "$REPOS/dirty" || say 'PROMPT B10 cd FAILED'
GIT_PS1_SHOWDIRTYSTATE=1 GIT_PS1_SHOWUNTRACKEDFILES= GIT_PS1_SHOWSTASHSTATE= GIT_PS1_SHOWUPSTREAM=
PS1='PROMPT> '
__git_ps1
say "PROMPT B10 ps1 rc=$? ps1=[$PS1]"
say 'ENDPROMPT B10'

# B11: __git_ps1 outside any repo.
mkdir -p "$REPOS/norepo"
cd "$REPOS/norepo" || say 'PROMPT B11 cd FAILED'
out11=$(__git_ps1 '(%s)')
say "PROMPT B11 norepo rc=$? out=[$out11]"
say 'ENDPROMPT B11'

# B12: pcmode (2 args) sets PS1 in dirty repo.
cd "$REPOS/dirty" || say 'PROMPT B12 cd FAILED'
GIT_PS1_SHOWDIRTYSTATE=1 GIT_PS1_SHOWUNTRACKEDFILES= GIT_PS1_SHOWSTASHSTATE= GIT_PS1_SHOWUPSTREAM=
PS1='ORIGINAL '
__git_ps1 'PROMPT> ' ' \$ '
say "PROMPT B12 pcmode rc=$? ps1=[$PS1]"
say 'ENDPROMPT B12'

say 'DRIVER DONE'
