x=$'\xff'
printf 'L1 |'; printf '%s' "${x:0:1}" | od -An -tx1
printf 'L1b|'; printf '%s' "${x:1}"   | od -An -tx1
y=$'a\xffb'
printf 'L1c|'; printf '%s' "${y:2}"   | od -An -tx1
