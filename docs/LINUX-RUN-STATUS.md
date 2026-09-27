# Linux Run Status (lane wt4/linuxrun)

> Lane: Linux-port verification, 2026-09-27. Worktree `D:/repo/rubash-wt-linuxrun`,
> branch `wt4/linuxrun`, base `9bf2df9e` (= master tip at lane start) plus the
> three lane fixes listed below. Baseline: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`),
> coreutils 9.4, kernel 5.10.16.3-microsoft-standard-WSL2, running everything
> (both shells) inside the same WSL instance — pure engine diffs, zero Windows
> noise. Raw artifacts: `target/wslrun/` (probes + smoke results) and
> `target/issue-suites/results/true-baseline/` (corpus).

## Verdict

**The Linux target builds and runs for real.** `cargo build --target
x86_64-unknown-linux-gnu` green (59.7 s cold, 40.7 s after edits), ELF binary
works as a script shell: PATH lookup, exit-status decode, traps, kill,
pipes/globs/arith/functions/`$(...)` all byte-match GNU on the smoke matrix.
The prior "not-yet-usable" judgment (2026-09-26) is obsolete: the unix
`cfg` arms landed since then cover PATH two-tier resolution, `faccessat`-class
exec bits, signal tables, `process_exit_status`, and a real `signal_hook`
backend. What remains are two suite-level HANGS (trap, redir — which the
SA_RESTART signal backend makes unkillable, hence visible), locale-collation
glob ordering, and a set of small diagnostic-shape gaps. User hypothesis
CONFIRMED: the Linux side exposes fewer platform issues (no WinuxCmd/DrvFS
noise at all — `env=0` on every suite) and pure engine bugs.

## Smoke matrix (script files run through both shells, diff stdout+stderr+rc)

| Probe | Class | Result after lane fixes |
|---|---|---|
| smoke_basic | echo/printf/vars/quoting/special params/arrays/printenv | PASS (only `$$` pid line differs — probe artifact) |
| smoke_loops | for/while/until/case/break/continue/`$()` in for | PASS byte-identical |
| smoke_funcs | functions/args/return/local/recursion/nested | PASS byte-identical |
| smoke_pipes | pipelines/pipefail/heredoc/redirect/tmpfiles | PASS byte-identical |
| smoke_glob | glob/`?`/`[]`/braces/nullglob/extglob | PASS byte-identical |
| smoke_arith | arithmetic incl. `let`, bases, side effects | PASS byte-identical |
| smoke_subst | `$(...)`, backticks, nesting, assignment RHS | PASS byte-identical |
| p0_path | `command -v`/`type`/`/bin/echo`/restricted PATH/`command -p` | PASS byte-identical |
| p0_status | exit-code decode, 128+N for TERM/KILL/INT/HUP/QUIT/USR1, `wait` decode | PASS stdout; stderr: missing signal-death notices (#229) |
| p0_execbit | chmod 644→126, chmod +x→run, 744 | PASS after chmod fix (was: chmod was a no-op) |
| p0_signal | traps EXIT/TERM/USR1, kill builtin, `kill -0`, fg signal status | FAIL: `kill -l 32` (#230), wait race (#228), notices (#229) |
| p1p2_identity | BASH_VERSION/OSTYPE/MACHTYPE/uname/jobs -l/set -m | PASS stdout after uname+OSTYPE fix except jobs -l spacing (#233); stderr: set -m noise (#234) |

Extra probes: `probe_subshell.sh` (subshell `$$` = main pid, trap inheritance)
byte-identical; `probe_nvm.sh`: **nvm.sh v0.40.3 (4661 lines) parses clean
(`-n` rc=0), `. nvm.sh --no-use` loads `nvm` with byte-identical stderr,
`nvm_version` rc=0** — the whole real-world script shape works.

## Live P0/P1/P2 status (vs the 2026-09-26 assessment)

| Prior gap | Live status at 9bf2df9e (+lane fixes) |
|---|---|
| P0 PATH injection / tool dispatch | **RESOLVED** — `find_user_command` unix walk with exec-bit preference + `file_to_lose_on` (E6 gate in place); `/bin/ls`, `command -v`, `command -p` byte-match |
| P0 wait/exit decode | **RESOLVED for codes** — `wait_status::process_exit_status` unix arm; 128+N everywhere incl. `wait` on signaled children. Residual: wait-interrupt race (#228), stderr notices (#229) |
| P0 exec-bit | **RESOLVED in resolution logic** (126 vs 127 correct); **root blocker was chmod**: the emulated chmod never called chmod(2) on unix — FIXED this lane. `test -x` also fixed (was exists()-always-true) |
| P0 signals | **MOSTLY RESOLVED** — signal_hook backend (INT/TERM/HUP/QUIT/USR1/USR2), libc signal tables, trap dispatch. Residual: SA_RESTART unkillable-block (#226), `kill -l 32/33` (#230) |
| P1 job groups/termios | **STILL ABSENT** — zero setpgid/tcsetpgrp call sites; observable as `set -m` stderr noise (#234). Script-mode job bookkeeping (jobs/wait/%) works without them |
| P2 version strings | msys triple leak FIXED (bash --version line 1 correct). Left: engine naming in tool banners + missing Copyright block (#240). uname identity on unix FIXED this lane (was lowercase/unknown/localhost) |

## Lane fixes applied (small cfg(unix) changes, both targets green)

1. `src/executor/identity.rs` — unix arms backed by `libc::uname` (utsname):
   `sysname`/`release`/`kernel_version`/`nodename` verbatim, `-o` =
   `GNU/Linux`, `-p`/`-i` = utsname.machine (coreutils 9.4 probe), `ostype()`
   = `linux-gnu` on glibc (variables.c:724 `set_if_not ("OSTYPE", OSTYPE)`).
2. `src/builtins/uname.rs` — `-a` includes the p/i slots on unix (coreutils
   field order s n r v m p i o); test made cfg-aware.
3. `src/executor/external_file_builtins.rs` — `external_chmod` unix arm:
   real `chmod(2)` via `PermissionsExt::set_mode`, base = real inode mode;
   emulated store stays Windows-only.
4. `src/builtins/test.rs` — `-r/-w/-x` on unix via
   `faccessat(AT_FDCWD, path, mode, AT_EACCESS)` (test.c:553/556/559 →
   lib/sh/eaccess.c:194 `sh_eaccess`).

Gates: `cargo check --all-targets` clean on Windows; `cargo check
--all-targets --target x86_64-unknown-linux-gnu` clean; lib tests 479/479
(Windows) and 419/419 (Linux).

## Corpus slice (true-baseline harness, Linux binary as RUB_OVERRIDE, all inside WSL)

Method note: the harness's default `RUBSIDE_PATH` forwards the **Windows**
PATH (`/mnt/c/...`) for a Windows binary; for the Linux binary it must be
`$BASE:/usr/local/bin:/usr/bin:/bin`. First pass without this starved
`/usr/bin:/bin` — globstar 154 / set-e 75 / more-exp / exp / case / procsub
diff lines were 100% PATH starvation, not engine diffs. With the corrected
PATH (ledger2):

| Suite | stdout diff lines | Class |
|---|---|---|
| arith, arith-for, more-exp, exp, comsub, comsub2, dstack, dstack2, jobs, heredoc, case, braces, cond, globstar, procsub, printf, set-e | **0** (17 suites byte-identical) | — |
| trap | 30 (+ rc 137) | HANG after trap4.sub (#227) |
| redir | 58 (+ rc 137) | HANG in redir7.sub backquote comsub (#225) |
| glob | 40 | collation ordering (#235) + escaped-star leak (#236) |
| extglob | 16 | collation ordering (#235) |
| new-exp | 6 | `${!a[@]}` literal (#239) |
| read | 2 | `read -t` output shape (#238) |
| ifs-posix | 1 (+ rc 124) | timeout/hang (#237) |

Plus nvm.sh (external to the suite corpus): full parse + load + version call
clean (see above).

## Issues filed from this lane

#225 redir hang · #226 unkillable SIGTERM block · #227 trap.tests hang ·
#228 wait race · #229 signal-death notices · #230 kill -l 32/33 ·
#231 exec-failure prefix · #232 mktemp suffix · #233 jobs -l spacing ·
#234 set -m noise (P1 layer) · #235 collation ordering · #236 escaped-star
glob · #237 ifs-posix timeout · #238 read -t · #239 `${!a[@]}` ·
#240 version-string leaks.
