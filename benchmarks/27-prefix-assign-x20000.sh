v=outside
for ((i=0;i<20000;i++)); do v=$i :; done
printf "%s %s\n" "$v" "$i"
