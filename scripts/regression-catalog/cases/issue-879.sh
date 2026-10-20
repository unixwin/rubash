# issue rubash#879 - printf float faces: %f/%e/%g must expand with
# long-double precision (1e999 is finite there, not inf) and the %a/%A
# mantissa style must match glibc.  Verified byte-identical to the GNU
# oracle; the manifest row is must-match (promoted per #949) - if this
# ever diverges again, it is a regression.  Values are captured into a
# variable first so a builtin never writes through a closing pipe.
r=$(printf '%f\n' 1e999); echo "f999-len=${#r} pre=${r:0:24}"
r=$(printf '%f\n' 1e400); echo "f400-len=${#r} pre=${r:0:24}"
r=$(printf '%e\n' 1e999); echo "e999=$r"
r=$(printf '%a\n' 1e999); echo "a999=$r"
printf '%a\n' 5
printf '%A\n' 5
printf '%a\n' 0.1
printf '%.3a\n' 5
