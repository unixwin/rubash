f=fdtest
i=0
while [ $i -lt 1200 ]; do
  exec {fd}>"$f" || break
  i=$((i+1))
done
echo "allocated=$i"
