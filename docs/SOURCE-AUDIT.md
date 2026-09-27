# GNU C Source vs Rust Owner — Line-Level Gap Audit (wt5/audit, 2026-09-28)

Baseline: worktree `wt5/audit` @ `f6023db8` (rust build verified before measuring).
Oracle: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`), script-file probes only.
Harness: `target/issue-suites/results/source-audit/cmp.sh NAME` (runs
`target/debug/rubash.exe probes/NAME.sh` vs `wsl bash probes/NAME.sh`,
captures stdout/stderr/exit separately, unified diff). Raw artifacts:
`target/issue-suites/results/source-audit/{probes,out}/`. Durable reproducers:
`tests/fixtures/audit/`.

Method per area: (a) inventory C functions/tables; (b) locate rubash Rust
owner; (c) classify IMPLEMENTED / MISSING / DIVERGENT / N-A(platform);
(d) every MISSING/DIVERGENT verified with a both-shell probe before filing.

Citations are `file:line func` in `third_party/bash/` (vendored
Bash-5.3 patch 15 tree; re-verify with grep before relying on them).

---

## Summary counts

| Area | behaviors probed | IMPLEMENTED | DIVERGENT | MISSING | N-A / audit-only |
|---|---|---|---|---|---|
| 1. builtins option tables | 96 | 66 | 4 (issues #261 #270 #272 + minor) | 1 family (ulimit, #261) | 2 platform |
| 2. redir.c redirection forms | 40 | 32 | 4 (#263 #264 #265 #266) | 0 | 2 platform |
| 3. subst.c parameter operators | 74 | 74 | 0 | 0 | 0 |
| 4. parse.y grammar productions | 46 | 42 | 2 (#268 #269) | 0 | 1 reprint cosmetic |
| 5. jobs.c/trap.c display | 22 | 18 | 1 filed-adjacent (#233 comment) | 0 | 3 job-control/platform |
| 6. variables.c attributes | 58 | 53 | 3 (#267 #271 #272) | 0 | 2 minor |
| **total** | **336** | **285** | **14** | **1** | **10** |

Issues filed: #261 #263 #264 #265 #266 #267 #268 #269 #270 #271 #272
(11 new) + evidence comment on #233. No src/ changes (read-and-probe lane).

Top-3 impact: (1) #261 ulimit is a stub — every query/set/exit code wrong;
(2) #266 `<&badfd` silently runs commands with the wrong stdin
(data-corruption class); (3) #267 `declare -i` arith error executes code GNU
discards (control-flow divergence).

---

## Area 1 — builtins option tables (`third_party/bash/builtins/*.def`, 43 files)

Inventory: `internal_getopt` strings extracted for all 43 .def files
(alias `p`; unalias `a`; bind `lvpVPsSXf:q:u:m:r:x:`; cd `eLP`; exec `cla:`;
fc `:e:lnrs`; printf `v:`; enable `adnpsf:`; suspend `f`; mapfile
`d:u:n:O:tC:c:s:`; complete `abcdefgjko:prsuvA:G:W:P:S:X:F:C:V:DEI`,
compopt `+o:DEI`; command `pvV`; declare `+acfgilnprtuxAFGI`; read
`Eersa:d:i:n:p:t:u:N:`; ulimit table `limits[]`; hash `dlrp:rt`; wait
`fnp:`; type `afptP`; help `dms`; trap `lpP`; umask `Sp`; history
`acd:npsrw`; jobs `lpnxrs`, fg/bg `ahr`; setattr `aAfnp`; shopt `psuoq`;
source `p:`; set builtin arg-parse `fnv`).

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| complete.def:112-135 compacts[] (24 -A actions), :144-152 compopts[] (9 -o names), :205/:827 getopt strings | src/builtins/complete.rs COMPACTS/COMPOPTS + flag_options/arg_options | IMPLEMENTED (tables byte-equal incl. order) | a1 set — no diff spot-checks; table diff vs C source | — |
| trap.def:121 getopt `lpP`; common.c:230 sh_invalidsig; trap --/-p/-l/EXIT passthrough | src/builtins/trap.rs | IMPLEMENTED | a1-trap SAME stdout, stderr path-prefix only | — |
| set.def:673 option name table; shopt.def:303 `psuoq` + shopt name table | src/builtins/set.rs, src/builtins/shopt.rs | IMPLEMENTED (full `set -o` + `shopt -p` listing byte-identical; invalid-name errors match) | a1-setopt SAME | — |
| declare.def:145 `+acfgilnprtuxAFGI`; attr print order; -I MKLOC_INHERIT; nameref self/cycle | src/builtins/declare.rs | IMPLEMENTED for print formats/attrs; DIVERGENT for `readonly -p NAME` filter (setattr.def owner) | a1-declare DIFF (readonly -p section) | #272 |
| read.def:308 `Eersa:d:i:n:p:t:u:N:` edge table: -t abc/-1/0/0.5, -n/-N abc/-1/0, -u abc/99/-1, -d ''/delim, -i, -r, IFS='' | src/executor/read_builtin.rs | IMPLEMENTED except two rows | a1-read-edge + a1-read-d0 DIFF (rows A,B) | #270 |
| read.def:949 `retval = eof ? EXECUTION_FAILURE : SUCCESS` (the -d '' EOF row) | src/executor/read_builtin.rs | DIVERGENT (rc 0 vs 1) | a1-read-d0 case A | #270 |
| read.def:332-336 `-i` only under READLINE edit mode (-e/-E) | src/executor/read_builtin.rs | DIVERGENT (seeds var without -e) | a1-read-d0 case B | #270 |
| ulimit.def:239 limits[] (P R T b c d e f i k l m n p q r s t u v w x), :364 getopt, sh_invalidnum rc matrix | src/builtins/ulimit.rs (stub, TODO :28) | MISSING/DIVERGENT across the board | a1-ulimit DIFF | #261 |

Audit-only (not filed):
- Circular-nameref warning count: GNU prints `warning: c1: circular name
  reference` 3x (echo + unset c1 + unset c2 derefs) vs rubash 1x
  (a1-declare stderr). Cosmetic, low value as a standalone issue.
- `declare -p` of BASH_VERSINFO carries `x86_64-pc-msys` host triple —
  platform identity, N-A.

## Area 2 — redir.c redirection forms (20 opcodes)

Inventory: `r_input_direction, r_output_direction, r_appending_to,
r_inputa_direction, r_input_output, r_output_force, r_reading_until,
r_deblank_reading_until, r_reading_string, r_duplicating_input,
r_duplicating_input_word, r_duplicating_output, r_duplicating_output_word,
r_move_input, r_move_input_word, r_move_output, r_move_output_word,
r_err_and_out, r_append_err_and_out, r_close_this`. Rust owner:
`src/parser/redirections.rs` → `src/executor/redirection.rs`.

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| make_cmd.c:682 `r_input_output` → `O_RDWR\|O_CREAT` | src/executor/redirection.rs rw-open | DIVERGENT: no create on missing file | a2-iso case 1 | #264 |
| make_cmd.c:704-718 `>&word-`/`<&word-` → r_move_output/input(_word); redir.c:1253 cases | src/parser/redirections.rs operand classification | DIVERGENT: parsed as literal filename `&5-` | a2-iso cases 3-4 | #263 |
| redir.c:895-930 redirectee_word==0 only for multi-word; redir.c:135-197 redirection_error prints EXPANDED name (open errors) / RAW word (dup EBADF) | src/executor/redirection.rs:76,:118-156 | DIVERGENT: empty expansion → `ambiguous redirect`, prints quoted raw operand | a2-iso case 5 | #265 |
| redir.c:1115 r_duplicating_input dup failure + redir.c:149-158 `itos(redirectee)` EBADF report | src/executor/redirection.rs input-dup arm | DIVERGENT: `<&7` silently ignored, command runs rc=0 | a2-iso case 2 | #266 |
| `>` `>>` `<<` `<<-` `<<<` `>\|` noclobber + force, `&>` `&>>`, `>&n` `<&n` `<&-` `>&-`, `{var}<>` exec-fd alloc + `read -u` round-trip, `>&1a` digit/word rule, missing-file/perm/noclobber errors, multi-fd exec open/close | src/executor/redirection.rs | IMPLEMENTED (32 forms byte-identical incl. exit codes and diagnostics) | a2-redir-forms + a2-iso controls | — |

Audit-only (not filed):
- `cat <&-`: GNU hands the child a genuinely closed fd 0 (external `cat`
  fails `Bad file descriptor`, rc=1); rubash substitutes a readable empty
  stdin (rc=0). Platform-entangled (child fd inheritance on Windows);
  audit-documented.
- `&>` capture of an external tool's message differs in bytes (62 vs 71) —
  WinuxCmd `ls` message length, N-A(platform), not a rubash bug.

## Area 3 — subst.c parameter operator corner table

C anchors: `subst.c:8740 string_transform()` (a A K/k E P Q U u L),
`subst.c:9777 parameter_brace_expand()`, `subst.c:7663
parameter_brace_expand_word()`, substring/pattern arms in the same file.

74 behaviors probed in `a3-param-ops.sh`: every @-transform incl.
undocumented `@k` and `@a`; @Q/@E/@P/@A over scalars/int/indexed/assoc
vars; `@@Q` vs `*@Q`; `${!p@}`/`${!p*}` prefix listing; `${!q}` indirect +
`@Q`; substring edges (`:6` `:0:5` `:-5` `:0:-5` `:-5:2` `:100` `:0:100`
`:1.5` `:0:0` `:0:-1` `:0:-99`); pattern ops under nested quotes
(`#"` `#\"a` `%"` `#'` `%%'*`); extglob patterns in `${v#+([a-z])}` /
`${v//@([a-z])/X}`; case mods `^ ^^ , ,, ~ ~~ ^h ^[a-z] ^^[aeiou] ,l
,,[lr]`; `${///}` replacement specials (`\$`, `&`, `\&`, `\\`, quoted
operator); array subscripts (`[@]:1`, `[@]:0:1`, `[@]:-1`, scalar `:1`);
`${#v}`/`${#av[@]}`/`${#As[@]}`; full `:- - = + ? :?` unset family under
`set -u` incl. error text and exit codes.

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| subst.c:8740 string_transform (all 11 operators) | src/expand/ (param op arms) | IMPLEMENTED | a3-param-ops SAME (stdout byte-identical; stderr path-prefix only) | — |
| subst.c:9777 parameter_brace_expand corner table (substrings, patterns, case, alternates, indirect/prefix forms) | src/expand/ | IMPLEMENTED | a3-param-ops SAME | — |

Zero divergences found in this area on this battery — the highest-risk
per AGENTS.md carrier rules held under the corner table.

## Area 4 — parse.y grammar productions

C anchors: `parse.y:575-640` redirection productions; function_def /
coproc / select / case productions (`parse.y` grammar section);
`execute_cmd.c:1244-1245` TIMEFORMATs, `:3426 select_query`,
`:3495 execute_select_command`.

46 behaviors probed in `a4-grammar.sh`: three function forms + args; named
(`coproc COP` + `COP[0]`/`COP_PID`) and anonymous coproc; select with piped
answer and with EOF stdin; case shapes (`es` literal pattern, `e?`, empty
body, esac-only, `;;&`, `;&`); `time` over simple command / brace group /
subshell; `TIMEFORMAT='%R'`; `time -p`; compound redirects on `{}`/`for`/`if`;
process substitution `<( )` incl. `diff`; arithmetic command + `((x+=3))`;
nested subshells; `until`; `!` negation; and-or chains; brace-group
`2>&1 1>/dev/null` ordering; `{ ; }` syntax error (both agree);
`let`; nested backticks; multiline quoted assignment.

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| execute_cmd.c:1245 BASH_TIMEFORMAT default (\`\\nreal\\t%3lR...\`) vs :1244 POSIX_TIMEFORMAT (only `time -p`) | src/executor time implementation | DIVERGENT: default time always POSIX format | a4-grammar stderr DIFF | #268 |
| execute_cmd.c:3426 select_query EOF `putchar('\\n')` + :3577 retval=EXECUTION_FAILURE | src/executor select loop | DIVERGENT: rc=0, no stdout newline | a4-grammar stdout DIFF | #269 |
| all other productions above | src/parser/ + src/executor/ | IMPLEMENTED (42/46) | a4-grammar remaining sections SAME | — |

Audit-only (not filed): syntax-error context reprint normalizes whitespace —
GNU reprints the offending source line verbatim (`printf ' a  b \n' | {IFS=`),
rubash drops the space before `|` (`' | {` → `'| {`). Cosmetic reprint
divergence; adjacent to #204 (wrong-line reprint) but a different mechanism
(token-reserialized line vs original source text).

## Area 5 — jobs.c / trap.c display semantics

C anchors: `jobs.c:2046 print_pipeline()` (`%5ld` pid at :2066, state-word
padding, `(core dumped)`), `jobs.c:2172 pretty_print_job()`,
trap.def listing format.

22 behaviors probed in `a5-jobs.sh` / `a5-iso.sh`: `jobs` default format;
`-l` `-p` `-r` `-s` `-x`; done-state after `wait`; stopped-job display;
`wait %99` message + rc; `wait -n`; `disown`; `fg %5`; `jobs %1`;
trap -l/-p/`--`/EXIT-passthrough (area 1 probe).

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| jobs.c:2066 `fprintf("%5ld", pid)` in long format | src/executor/job_builtins.rs | DIVERGENT (no width-5 padding; same string as #233 marker-spacing) | a5-jobs: GNU `[1]+    23 Running`, rubash unpadded | #233 (comment with evidence) |
| jobs.c pretty_print_job/print_pipeline state words, `-p`, `-r/-s` filters, `-x` replacement, wait %99/disown/fg errors | src/executor/job_builtins.rs | IMPLEMENTED | a5-jobs SAME modulo pid values | — |

Audit-only (not filed, job-control/platform territory per AGENTS.md):
- Stopped-job state freshness: after `kill -STOP $!; sleep`, GNU `jobs`
  still prints `Running ... &` (SIGCHLD state applied lazily; verified
  twice, `ps` shows state T while `jobs` says Running), rubash prints
  `Stopped` immediately (polls). Deterministic GNU behavior; rubash is
  "more accurate" but not byte-compatible.
- `wait $stopped_pid`: GNU blocks indefinitely; rubash returns 1
  immediately. Deadlock-vs-progress divergence; scripts cannot match both.
- SIGCHLD notice text via external `bash -c` indirection — environment
  noise (`bash` on PATH is the shim), N-A.

## Area 6 — variables.c attribute semantics

C anchors: `variables.c:2904 make_variable_value()` (+ :2938-2944
`jump_to_top_level (DISCARD)`), `variables.c:3386 bind_int_variable()`,
declare.def attribute matrix, setattr.def readonly/export arms,
set.def:964 unset readonly diagnostic.

58 behaviors probed in `a6-vars.sh` + isolations: nameref to array/assoc
ELEMENT (read/write/unset); integer coercion rows; bases 2/16/36/64 +
negative base constant; indexed↔assoc conversion errors; readonly assign /
declare / unset / export interplay; nameref cycles through unset; `+=` on
scalar/indexed/int/assoc; `declare -g`; integer attr through nameref;
`declare -i` on pre-existing string value; `unset -v` vs `-f`.

| C anchor | rubash owner | classification | probe verdict | issue |
|---|---|---|---|---|
| variables.c:2938-2944 arith-eval failure → DISCARD (var stays unset, rest of line skipped) | src/builtins/declare.rs assignment path | DIVERGENT: assigns 0, continues line | a6-arith-err DIFF | #267 |
| set.def:964 `cannot unset: readonly variable` via builtin_error (honors fd-2 redirects) | unset builtin error arm | DIVERGENT: writes to original stderr, ignores `2>&1`/`2>/dev/null` | a6-unset-ro2 DIFF | #271 |
| setattr.def readonly -p NAME filter | src/builtins/setattr.rs | DIVERGENT: dumps all readonly vars | a1-declare/a6-vars DIFF | #272 |
| all other attribute rows above | src/builtins/declare.rs, setattr.rs, variables owner | IMPLEMENTED (53/58) | a6-vars remaining sections SAME | — |

Audit-only (not filed): circular-nameref warning multiplicity (GNU 3x vs
rubash 1x, a6-vars stderr) — same family as the area-1 note.

---

## Issue ledger from this lane

| issue | area | one-line summary |
|---|---|---|
| #261 | 1 | ulimit is a hardcoded stub (no getrlimit/setrlimit, truncated `-a`, wrong exit codes, unconditional `-u` EPERM) |
| #263 | 2 | `>&n-` / `<&n-` move-fd redirects parsed as literal filenames |
| #264 | 2 | `<> file` does not create the missing file (O_RDWR\|O_CREAT) |
| #265 | 2 | empty redirect expansion misreported as `ambiguous redirect`; wrong operand text |
| #266 | 2 | `<&7` bad input fd silently ignored (command runs, rc=0) |
| #267 | 6 | `declare -i` arith error: assigns 0 + continues; GNU leaves unset + aborts rest of line |
| #268 | 4 | `time` default format is POSIX style; GNU BASH_TIMEFORMAT (blank line + tabs + `0m0.000s`) |
| #269 | 4 | select on EOF: rc=0 and missing stdout newline (GNU rc=1 + newline) |
| #270 | 1 | `read -d ''` EOF status; `read -i` without `-e` applies initial text |
| #271 | 6 | `unset readonlyvar` diagnostic bypasses fd-2 redirection |
| #272 | 1/6 | `readonly -p NAME` ignores name filter, dumps all readonly vars |
| #233 (comment) | 5 | addendum: `%5ld` pid padding in `jobs -l` long format |

## What this audit did NOT cover (honest gaps)

- bind.def (readline bindings), mapfile/getopts deep matrices, fc/history
  editing, enable/loadable builtins: option strings inventoried, no probe
  battery (budget).
- redir.c `/dev/tcp|udp` network redirections: N-A on Windows by design.
- jobs.c interactive notification cadence (notify/notifyall modes),
  SIGCHLD trace messages: only the scripted subset probed.
- variables.c: dynamic vars (FUNCNAME/BASH_ARGV chains) beyond what other
  lanes already cover; nameref-to-special-var corner cases.
