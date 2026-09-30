shopt -s compat31
re='a+b'
v='xa+b'
[[ $v =~ "$re" ]] && echo m || echo nm
