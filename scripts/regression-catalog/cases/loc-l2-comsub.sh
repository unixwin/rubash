printf '\xff\xfe\x41\x80' > f
x=$(cat f); printf 'L2 |len=%s|' "${#x}"; printf '%s' "$x" | od -An -tx1
y=$(<f);    printf 'L2b|len=%s|' "${#y}"; printf '%s' "$y" | od -An -tx1
