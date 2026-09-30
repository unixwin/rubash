f() { return 5; }
declare -ft f
trap 'echo R' RETURN
f
echo "rc=$?"
