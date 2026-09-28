#!/usr/bin/env bash
# rubash#296: assignment RHS that expands BOTH a variable whose value holds a
# control byte stored as the U+E000+U+E0xx carrier pair AND an inline command
# substitution must store the raw byte, not the carrier (and not the
# U+E400-escaped carrier after a second pass). GNU anchor: quoted-context
# expansion output is re-escaped uniformly by quote_string (subst.c:4773 via
# the add_quoted_string label at subst.c:11862) and restored by dequote_string
# (subst.c:4807) at the assignment store boundary -- no character class is
# frozen mid-encode. The golden pins the raw stdout bytes of printf (od is not
# available in the isolated regression PATH): a correct value prints the raw
# ESC byte (0x1b) while a leaked carrier prints UTF-8 PUA bytes (ee 80 80 ee
# 80 9c) and a second pass escalates to the E400-escaped form -- all distinct
# at the byte level. The ${#...} length echoes are a readable secondary
# signal: the decoded length is 11; a leaked pair lengthens it.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash issue296-carrier-adjacent-comsub.sh' \
#     > issue296-carrier-adjacent-comsub.gnu.out
v=$'\e[38;5;248m'
# quoted prefix / suffix / both sides
a1="${v}$(true)";           printf '%s\n' "$a1" "${#a1}"
a2="$(true)${v}";           printf '%s\n' "$a2" "${#a2}"
a3="${v}$(true)${v}";       printf '%s\n' "$a3" "${#a3}"
# unquoted adjacency
a4=${v}$(true);             printf '%s\n' "$a4" "${#a4}"
# comsub with output
a5="${v}$(echo hi)";        printf '%s\n' "$a5" "${#a5}"
# backtick adjacency
a6="${v}"`true`;            printf '%s\n' "$a6" "${#a6}"
# second pass over an already-stored mixed value
b1="${v}$(true)"; a7="${b1}$(true)"; printf '%s\n' "$a7" "${#a7}"
# payload byte 0x11 rides the same transport (raw 0x80 and literal U+E000
# outputs are excluded: those pin the stdout encoding boundary, a different
# owner from this assignment-store fix; the shapes are verified against GNU
# in the lane probe matrices instead)
n=$'\x11';   a8="${n}$(true)";   printf '%s\n' "$a8" "${#a8}"
# PS1 shape from the issue: color var + comsub + reset var
fg=$'\e[38;5;220m'; rs=$'\e[0m'
ps="${fg}$(printf 'X')${rs}"
printf '%s\n' "$ps" "${#ps}"
ps2="${ps}$(true)"
printf '%s\n' "$ps2" "${#ps2}"
# controls: no comsub / comsub via var / argument position
c1="${v}x";                 printf '%s\n' "$c1" "${#c1}"
cv="$(true)"; c2="${v}${cv}"; printf '%s\n' "$c2" "${#c2}"
printf '%s\n' "${v}$(true)"
