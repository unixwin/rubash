#!/usr/bin/env bash
# Background-signal trap matrix (commit 7ddae9e9): {grp}& / (subshell)& /
# fn& x TERM/INT, untrapped 143, trap '' survival, trap-exit status,
# trap/next-command ordering, no-wait variant. GNU model: trap_handler only
# RECORDS a caught signal while a command runs (trap.c:537-543) and
# run_pending_traps fires it at the next command boundary
# (execute_cmd.c:643); an untrapped terminating signal kills the shell
# (jobs.c:3064 wait_for). Verified byte-identical against WSL GNU Bash
# 5.3.0 on 2026-09-27.
# Capture (run from a scratch copy of this directory):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash trap-signal-matrix.sh' \
#     > trap-signal-matrix.gnu.out 2> trap-signal-matrix.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
echo "== grp-term-wait"
{ trap "echo got TERM" TERM; sleep 2; } & sleep 1; kill $!; wait; echo "rc=$?"
echo "== grp-int-wait"
{ trap "echo got INT" INT; sleep 2; } & sleep 1; kill -INT $!; wait; echo "rc=$?"
echo "== sub-term-wait"
( trap "echo sub TERM" TERM; sleep 2; ) & sleep 1; kill $!; wait; echo "rc=$?"
echo "== sub-int-wait"
( trap "echo sub INT" INT; sleep 2; ) & sleep 1; kill -INT $!; wait; echo "rc=$?"
echo "== fn-term-wait"
tfn() { trap "echo fn TERM" TERM; sleep 2; }; tfn & sleep 1; kill $!; wait; echo "rc=$?"
echo "== fn-int-wait"
tfin() { trap "echo fn INT" INT; sleep 2; }; tfin & sleep 1; kill -INT $!; wait; echo "rc=$?"
echo "== untrapped-term-143"
{ sleep 2; } & sleep 1; kill $!; wait $!; echo "rc=$?"
echo "== trap-empty-survives"
{ trap '' TERM; sleep 2; echo survived; } & sleep 1; kill $!; wait; echo "rc=$?"
echo "== trap-exit-status"
{ trap "echo bye; exit 7" TERM; sleep 2; } & sleep 1; kill $!; wait $!; echo "rc=$?"
echo "== trap-before-next-command"
{ trap "echo T" TERM; sleep 2; echo B; } & sleep 1; kill $!; wait; echo "rc=$?"
echo "== grp-term-nowait"
{ trap "echo nw" TERM; sleep 2; } & sleep 1; kill $!; sleep 2; echo "after"
echo done
