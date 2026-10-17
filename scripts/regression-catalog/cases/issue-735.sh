x2=$'\uE401'; printf 'E401 len=%s\n' "${#x2}"; printf '%s' "$x2" | od -An -tx1
x3=$'\uE301'; printf 'E301 len=%s\n' "${#x3}"; printf '%s' "$x3" | od -An -tx1
