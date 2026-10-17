printf 'abc\0def\0\n' > f
x=$(cat f); printf 'cmdsub_len=%s\n' "${#x}"
IFS= read -r z < f; printf 'read_len=%s\n' "${#z}"
