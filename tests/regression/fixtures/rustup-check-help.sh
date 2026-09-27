#!/usr/bin/env bash
# rustup-init.sh check_help_for shape (rubash#205, fixed by f15bacdd).
# The real installer probes downloader capabilities with
#   if "$_cmd" --help | grep -q '"--help all"'; then
# where the pipeline's second segment is a native exe whose argv embeds
# literal double quotes. rubash used to strip those quotes from native
# argv, flipping the probe's result. The stubs below discriminate: an
# unquoted `--help all' line must NOT match the quoted pattern, a quoted
# line must match.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash rustup-check-help.sh' \
#     > rustup-check-help.gnu.out 2> rustup-check-help.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
stub_unquoted() { printf 'usage: --help all (unquoted)\n'; }
stub_quoted() { printf 'usage: "--help all" (quoted)\n'; }
check_help_for() {
    local _cmd _category
    _cmd="$1"
    if "$_cmd" --help | grep -q '"--help all"'; then
      _category="all"
    else
      _category=""
    fi
    echo "category=${_category:-none}"
}
echo "== unquoted help text must not match the quoted pattern"
check_help_for stub_unquoted
echo "rc=$?"
echo "== quoted help text must match"
check_help_for stub_quoted
echo "rc=$?"
echo done
