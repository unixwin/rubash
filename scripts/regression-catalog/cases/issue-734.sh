k=$'x\ty'
declare -A A4; A4[$'x\ty']=1; printf 'A4=[%s]\n' "${A4[$k]}"
declare -A A5; A5[$'a\\b']=1; printf 'A5keys='; printf '%s' "${!A5[@]}" | od -An -c
h=X
b=('$h'); printf 'b0=[%s]\n' "${b[0]}"
