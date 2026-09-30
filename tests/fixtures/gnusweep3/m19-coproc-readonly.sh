declare -r RO RO_PID
coproc RO { :; }
declare -p RO_PID
wait
declare -p RO RO_PID
