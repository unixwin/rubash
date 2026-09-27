#!/usr/bin/env bash
# Ecosystem smoke 3/3: oh-my-bash theme loading + PS1 rendering. The full
# oh-my-bash corpus (22 libs + 5 themes) is not vendored (not on disk at
# codification time); this smoke pins the load-and-render shape every OMB
# theme uses: lib functions are sourced in, a theme function composes PS1
# with escapes + command substitution + $?-dependent glyphs, and the shell
# expands the composed prompt via ${PS1@P}. git output is stubbed by a
# function so no git binary is needed. Verified byte-identical against
# WSL GNU Bash 5.3.0.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash eco-omb-theme-ps1.sh' \
#     > eco-omb-theme-ps1.gnu
# --- lib layer (sourced shape) ---
_omb_util_print() { printf '%s\n' "$*"; }
_omb_prompt_get_git() {
  git() { echo ' (main)'; }
  _omb_branch=$(git branch 2>/dev/null)
  unset -f git
}
# --- theme layer ---
omb_theme_example() {
  local ec=$? glyph='>'
  ((ec == 0)) && glyph='$'
  _omb_prompt_get_git
  PS1='[theme]'"${_omb_branch}"' ${glyph} '
}
omb_theme_example
echo "== ps1 literal"
printf '%s\n' "$PS1"
echo "== ps1 rendered"
printf '%s\n' "${PS1@P}"
echo "== lib call"
_omb_util_print lib-ok
echo "== function survives"
type -t omb_theme_example _omb_util_print _omb_prompt_get_git
echo done
