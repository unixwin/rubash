# Respawn transport census (rubash#2058)

GNU's fork-shaped children — process-substitution bodies, async `&`
function/subshell bodies, coproc bodies (subst.c:6362
`process_substitute`, execute_cmd.c:1576 `execute_in_subshell`,
coproc `make_child`) — inherit the parent's `dollar_vars` and call
stack verbatim (variables.c). A respawned `rubash -c` child instead
re-imports only its process environment, so every piece of
fork-inherited state needs an explicit transport decision. This
census (probed against GNU bash 5.3.20 at script top level, inside a
function, and inside a function under `extdebug`) gives each item a
three-state verdict; the TRANSPORT rows are closed by one shared
mechanism at the four spawn sites, not per-site patches.

| Item | Verdict | Notes |
|------|---------|-------|
| Positional params | TRANSPORT (done, #2032) | `__RUBASH_POSITIONAL_PARAMS` |
| BASH_ARGV / BASH_ARGC frames + bottom | TRANSPORT | incl. the *unmaterialized* state: without extdebug a fork child read inside a function context stays empty; the bottom frame is the one frozen at materialization time, not the live window |
| FUNCNAME / BASH_SOURCE / BASH_LINENO stacks | TRANSPORT | child's views read its own frames above the inherited ones |
| LINENO start (body-text shapes) | TRANSPORT | the seed is the parent-source line of the body's first text line |
| LINENO in async-fn children | TRANSPORT, deferred | definition lines are destroyed by the reconstructed body text; needs source-span transport (see report) |
| SHLVL | TRANSPORT | a fork is not a new shell: no startup increment for internal respawns |
| PIPESTATUS | TRANSPORT | the value at fork time |
| $$ / BASHPID / $! / $PPID | PROJECT | projections per the #1582 ruling; the coproc site must stamp `__RUBASH_SHELL_PID` like the other sites |
| RANDOM sequence | DELIBERATELY-OMIT | GNU reseeds in a subshell environment (variables.c get_random_number); the contract is parent-untouched + child-fresh, which a fresh process already satisfies. Carrying the seed would break it |
| SRANDOM | DELIBERATELY-OMIT | entropy draws; GNU gives no stable inheritance contract |
| SECONDS, BASH_SUBSHELL, non-exported vars, locals (values), cwd, umask, $0 | TRANSPORT (done) | existing stamps / env merge, verified against the oracle |
