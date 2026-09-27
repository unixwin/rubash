#!/bin/bash
# minimal: stopped-job display + jobs -l pid padding + done-status text
sleep 5 &
P=$!
kill -STOP $P
sleep 0.3
jobs
jobs -l
kill -CONT $P
kill -TERM $P 2>/dev/null
wait $P 2>/dev/null
sleep 0.3
jobs
echo "== exit status text =="
sh -c 'exit 42' &
wait
jobs
echo "== jobs -s/-r after done =="
jobs -s
echo "s-rc=$?"
jobs -r
echo "r-rc=$?"
echo "== jobs %1 nonexistent =="
jobs %1; echo "rc=$?"
