false
PS1='x$?y'; printf 'A[%s]\n' "${PS1@P}"
MYVAR=xyz
shopt -s promptvars; PS1='A$MYVAR B'; printf 'B[%s]\n' "${PS1@P}"
shopt -u promptvars; printf 'C[%s]\n' "${PS1@P}"
echo "signals=$(compgen -A signal | wc -l) empty=$(compgen -A signal | grep -c '^$')"
