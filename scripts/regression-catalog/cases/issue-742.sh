p() { printf '[%s]' "$@"; printf '\n'; }
v=AB; vtb=QQ
p $v\tb
q=$v\tb; p "$q"
p ${v}\tb
w='a b'; wb=QQ
p a$w$'b'
p $w$'b'
v=AB
p $v\ b
p ${v}\ b
