mkdir -p d1 d2
pushd d1 >/dev/null; pushd ../d2 >/dev/null
echo "dirs-l=[$(dirs -l)]"
echo "dirs-p=[$(dirs -p | tr '\n' ' ')]"
shift 99; echo "shift99-rc=$?"
for b in setopt unsetopt; do printf 'type-%s=%s\n' "$b" "$(type -t $b 2>/dev/null || echo NONE)"; done
