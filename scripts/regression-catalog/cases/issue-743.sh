p() { printf '[%s]' "$@"; printf '\n'; }
w='a b'
p a$w"b"
p $w'b'
