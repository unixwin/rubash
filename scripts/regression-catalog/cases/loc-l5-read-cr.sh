printf 'a\r' > f1
IFS= read -r l < f1; printf 'L5 |len=%s|' "${#l}"; printf '%s' "$l" | od -An -tx1
printf 'a\r\r\n' > f2
IFS= read -r m < f2; printf 'L5b|len=%s|' "${#m}"; printf '%s' "$m" | od -An -tx1
