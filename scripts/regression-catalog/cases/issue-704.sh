printf '%d\n' '5 '; echo "printfd-rc=$?"
printf '%f\n' '5 '
command -v echo nosuch_xyz >/dev/null; echo "cmdv-multi-rc=$?"
OLDPWD=/foo; A=~+; B=~-; echo "tilde-assign A=[$A] B=[$B]"
echo "HISTCMD=[${HISTCMD-<unset>}]"
shopt -s extdebug; f(){ echo "fnargc=[${BASH_ARGC[*]}] n=${#BASH_ARGC[@]}"; }; f AA BB
