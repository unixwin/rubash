p() { printf '[%s]' "$@"; printf '\n'; }
p "$(cat <<< 'here')"
