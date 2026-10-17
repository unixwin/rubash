"$BASH" -c 'sleep 2; echo alive > marker' &
p=$!; sleep 0.3
kill -KILL $p
wait $p; echo "wait-rc=$?"
sleep 3
[ -f marker ] && echo SURVIVED || echo DIED
