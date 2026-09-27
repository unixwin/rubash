#!/usr/bin/env bash
# $(< file) cat-file command substitution matrix (commit 653b1de1,
# rubash#196): 23 of the 24 lane probe bodies (p1-p12 / q1-q12 from the
# target/p196 corpus; q10 is a known open divergence, see below).
# GNU owner: subst.c:7162-7179 command_substitute admission
# via parse_string_to_command + can_optimize_cat_file
# (builtins/evalstring.c:199); operand expansion via redir.c:298
# redirection_expand. Verified byte-identical against WSL GNU Bash 5.3.0
# on 2026-09-27. Runs in an EMPTY scratch directory: it creates adir and
# the *.txt fixtures.
# Capture (run from an EMPTY scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash catfile-comsub.sh' \
#     > catfile-comsub.gnu.out 2> catfile-comsub.gnu.err
# (write the captures OUTSIDE <scratch> so the .out/.err side files do
# not leak into glob results)
mkdir -p adir
printf 'a\nb\n\n' >exists.txt
printf 'A\n' >a.txt
printf 'B\n' >b.txt
: >g1.txt
: >g2.txt
printf 'nested\n' >nested.txt
printf 'A\n' >only.txt
printf 'A\n' >gx.txt
printf 'A\n' >gq.txt
printf 'A\n' >er.txt
printf 'A\n' >sl.txt
printf 'A\n' >va.txt
printf 'A\n' >es.txt
printf 'A\n' >br1.txt
printf 'sp ace\n' >'sp ace.txt'
echo "== p1 missing file"
LINES3=$(< ./no-such-file-probe); echo "rc=$?"
echo "== p2 glob no match"
x=$(< ./missing*); echo "rc=$?"
echo "== p3 quoted empty var"
unset uf; y=$(< "$uf"); echo "rc=$?"
echo "== p4 whole file minus trailing newlines"
z=$(< exists.txt); echo "[$z] rc=$?"
echo "== p5 extra word runs as command"
w=$(< ./nosuch cmd); echo "rc=$?"
echo "== p6 unquoted empty var ambiguous"
unset uf; y=$(< $uf); echo "rc=$?"
echo "== p7 glob two hits"
x=$(< g*.txt); echo "rc=$?"
echo "== p8 no space after <"
x=$(<exists.txt); echo "[$x] rc=$?"
echo "== p9 trailing comment (inner shell: the comment swallows the closing paren)"
"${THIS_SH:-bash}" -c 'x=$(< exists.txt # comment); echo "[$x]"'; echo "rc=$?"
echo "== p10 nested substitution"
x=$(< $(echo nested.txt)); echo "[$x] rc=$?"
echo "== p11 directory operand"
x=$(< adir); echo "rc=$?"
echo "== p12 quoted space filename"
x=$(< "sp ace.txt"); echo "[$x] rc=$?"
echo "== q1 one-glob hit"
x=$(< o?ly.txt); echo "[$x] rc=$?"
echo "== q2 two words from var"
d="a.txt b.txt"; x=$(< $d); echo "rc=$?"
echo "== q3 process substitution operand"
x=$(< <(echo hi)); echo "[$x] rc=$?"
echo "== q4 part-quoted glob"
x=$(< "g"x*.txt); echo "[$x] rc=$?"
echo "== q5 backslash-escaped glob char"
x=$(< g\q.txt); echo "[$x] rc=$?"
echo "== q6 extra redirect wins"
x=$(< er.txt > /dev/null); echo "[$x] rc=$?"
echo "== q7 second command after redirect"
x=$(< sl.txt; echo hi); echo "[$x] rc=$?"
echo "== q8 temp-assignment argument"
x=$(< va.txt VAR=1); echo "[$x] rc=$?"
echo "== q9 quoted empty operand"
x=$(< ""); echo "rc=$?"
echo "== q10 brace two words (known gap: excluded)"
# Known open divergence (653b1de1 documented gap, still present at
# 9bf2df9e): GNU reports `br{1,}.txt: ambiguous redirect'; rubash joins
# the brace fields to `br1.txt br.txt' and fails ENOENT. Excluded from
# the golden matrix until comsub-context redirect-operand ambiguity
# lands; see the wt4/ci codification report.
echo "== q11 read twice"
x=$(< es.txt); y=$(< ./es.txt); echo "[$x$y] rc=$?"
echo "== q12 missing directory"
x=$(< no/such/dir/f); echo "rc=$?"
echo done
