x='(a b c)'
printf '[%s] len=%s\n' "${x[@]}" "${#x[@]}"
set -- "${x[@]}"; echo "argc=$#"
y='plain'
printf 'control:[%s] len=%s\n' "${y[@]}" "${#y[@]}"
