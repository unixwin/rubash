q() { printf '%s\n' "$1"; }
b=`printf '%s' \\`; q "[$b] len=${#b}"
c=`printf '%s' "\\\\"`; q "[$c] len=${#c}"
d=$(printf '%s' \\); q "[$d] len=${#d}"
