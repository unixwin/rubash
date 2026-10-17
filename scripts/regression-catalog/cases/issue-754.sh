mkdir -p newhome
HOME="$PWD/newhome"; cd ~; echo "PWD-after-cd-tilde=$PWD"
for n in 220 260; do
  P="$PWD/$(printf 'a%.0s' $(seq 1 $n))"
  mkdir -p "$P" 2>/dev/null; m=$?
  (cd "$P" 2>/dev/null); c=$?
  echo "n=$n mkdir=$m cd=$c"
done
[ "$(pwd)" = "$(realpath .)" ] && echo equal || echo differ
