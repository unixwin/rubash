# Debugging scripts with `rubash --debug`

rubash ships a native debugger (rubash#497). One debugger, two
views:

- `rubash --debug script.sh [args…]` — the interactive terminal
  frontend described here.
- `rubash --debug --dap script.sh` — the same session spoken
  over the Debug Adapter Protocol, for editors (the zero-JS VS
  Code extension in `editors/vscode/` launches exactly this).

`--debug` with `-c`, `-s`, or no script keeps the older behavior
(the flag arms `extdebug`); the interactive frontend claims the
script-operand form. The debugger implies the `extdebug` gate
internally and never changes your observable `shopt extdebug`
setting.

The session starts stopped at the *entry stop*, before the first
command runs — set your breakpoints, then `continue` or `step`.

## Commands (M1 set)

| Area | Commands |
| --- | --- |
| Breakpoints | `break [line \| file:line \| function] [if cond]`, `tbreak`, `enable`/`disable`/`delete [id…]`, `clear [line \| file:line]`, `info breakpoints` |
| Stepping | `step`/`s` (into), `step+` (next seam in the same file), `step-` (next seam after leaving the current function), `next`/`n` (over), `finish`, `return [status]` |
| Inspection | `where`/`bt`, `frame N`, `up`, `down`, `list [line]`, `info locals\|variables\|args\|functions\|files\|state` |
| Evaluation | `print <expr>` (implicit power: the strict subset), `eval <command>` (explicit power: full shell) |
| Write-back | `eval` assignments, `set args <words…>`, `set var <name> = <expr>` |
| Exceptions | `exceptions [<filter> on\|off]`, `info exceptions` — filters: `err-trap`, `set -e`, `parse-error` |
| Lifecycle | `continue`/`c`, `quit`/`q` (EOF too), `kill` |
| Misc | `show listsize`, `set listsize N`, `tty` (report only), `help` |

The debugger's command stream is never the debuggee's stdin:
commands are read from the controlling terminal, the debuggee
keeps its own stdio, and everything the debugger prints goes to
stderr, so a piped stdout stays parseable.

## Semantics worth knowing

- **Breakpoints are keyed by canonical file identity.** Setting
  a breakpoint through a symlink, a relative path, or a Windows
  8.3 short name names the same entry as the file's real path.
  A line that cannot fire is reported *unverified* with the
  reason (`no command on this line`, or `DEBUG trap never fires
  on this line`); a target in a file not parsed yet is
  *pending* and resolves when the file loads.
- **`print` vs `eval` is a power line, on purpose.** `print`
  (and breakpoint conditions' implicit side, hover evaluation
  in editors) runs the strict subset: expansions and reads, no
  side effects — a rejected form says why and never executes.
  `eval` is full power: what you type runs in the stopped
  frame, including assignments, which is also how write-back
  works.
- **`quit` runs the exit path; `kill` does not.** `quit` (or EOF
  on the command stream) ends the session exactly as an
  un-debugged run would end at that point — EXIT trap included —
  carrying the debuggee's current status. `kill` terminates
  without the exit path and the run reports killed
  (status 128 + SIGTERM).
- **Exception stops are notifications, not vetoes.** When the
  error surfaces land (ERR trap site, an errexit decision, a
  parse error), the site is reported with its filter's name; the
  session does not cancel the exit it reports. Parking *at* the
  site before the trap runs is a later (M2) session option.

## Deliberate divergences from bashdb

- `step+` / `step-` are rubash's portable readings (bashdb's
  are zsh-flavored): `step+` stops at the next seam in the
  **current file**; `step-` stops at the next seam **leaving the
  current function**.
- There is no `load` (no second script runtime to extend; the
  extension points are command files via `source` and the
  session API both frontends expose), no `examine`/`search`
  (M2), and no display/watch CLI surface yet (watchpoints exist
  in the session core and in the DAP view; the CLI surface is
  M2).
- One thread, one session: background jobs are separate OS
  processes and cannot be attached; `attach` in editors is
  honored only in its launch-equivalent form.
