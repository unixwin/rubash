# Native Script Debugger — M0 Design (final)

> **FINAL — approved by the owner on 2026-10-25.** Document version:
> final (2026-10-25), superseding the v4 review draft of the same date.
> The §14 open questions Q1–Q5 are decided per the recommendations
> recorded there (§14 is now a decision record; §15 gains the
> finalization row). This design is finalized together with the #595
> Session API RFC as one 1.5.0 implementation program: the workspace
> split proceeds module by module, and each module's debug hooks land
> on the seams the split creates (§12 P-6, decided per §14 Q2).
>
> M0 design for rubash#497. This design **extends** the
> merged draft of record (`docs/design/debugger-m0.md`, PR #582) and
> the #497 consensus; it does not supersede or contradict either. It
> deepens the draft of record with the breakpoint / stepping /
> inspection semantics of §§5–10, and corrects an earlier working
> revision (v3) that had drifted from the owner's rulings on
> frontend order, flag semantics, and stop granularity. The ruling
> chain of record (full provenance in §15):
>
> - **2026-10-13** (issue comment, PR #582 merged): the draft of
>   record is adopted; DP1 (frontend order), DP2 (flag semantics),
>   DP3 (stop granularity) are put to the owner.
> - **2026-10-18** (owner ruling): DP1 = **DAP-first**; DP2 =
>   `--debugger` (GNU spelling, debugger-ready assembly); DP3 =
>   **line-level + function-entry**, two tiers; implementation
>   scheduled for 1.5.0, after the core split.
> - **2026-10-23** (owner ruling, final on flags): `--debug` *is* a
>   GNU option (a legacy one) and is adopted; `--dap` is likewise a
>   native citizen; **both flags are kept, with one option-parse
>   meaning, both naming the project's own native debugger**;
>   `--debugger` keeps the bashdb/GNU meaning, unmoved. This
>   supersedes the 2026-10-18 DP2 answer on the `--debug` point
>   only; DP1 and DP3 stand as ruled.
> - **2026-10-25** (owner, in review chat): the two frontends are
>   one thing — one session, two views; end users are expected to
>   live in `--debug` (the interactive CLI) once it lands. Recorded
>   as a usage expectation; it does not reorder DP1.
>
> Carried over unchanged from the #497 consensus and the draft of
> record: the seam substrate (§4), the ordering contract, evaluate
> safety (D4), gating (D5), the bashdb-oracle acceptance model (D6),
> watchpoint honesty (D2). Anchors re-verified at master `1e72c7f9`;
> the signal-trap (#1511) and procsub-lineno (#1184 f4) work is
> editing adjacent machinery this season, so **re-verify anchors
> when implementation opens**.

## 1. Positioning and goals

**Positioning.** bashdb is a bash script that debugs bash scripts from
outside the engine, using the only hooks GNU bash offers it (DEBUG
trap + extdebug + fabricated frames). Everything awkward about it —
per-command trap overhead on the debuggee, no watchpoints, fragile
frame bookkeeping, a second shell's worth of state to keep in sync —
is a consequence of that outsider position. rubash owns the engine, so
its debugger is built in: real stop seams, real frames, an inspection
surface that reads the executor's own state. The target is explicit:
**for any script a user would today run under bashdb-on-rubash, the
native `--debug` is the better tool, and bashdb remains only as
(a) the compatibility oracle and (b) the fallback for hosts where a
stock GNU bash is the debuggee.**

**Goals**

- A native debugger with the coverage of §10's table (the bashdb
  surface, item by item, each either matched, deliberately not
  matched, or phased — with the reason recorded): the DAP surface
  first (M0, §13) and the interactive gdb-grammar CLI,
  `rubash --debug script.sh`, immediately after (M1).
- One engine-owned session and event stream; both frontends are
  views over the identical semantics — the DAP adapter is the M0
  view (DP1, ruled 2026-10-18), the CLI lands immediately after.
  A behavior that exists in only one frontend is a bug by
  definition (§3).
- GNU fidelity wherever GNU or bashdb defines the observable
  behavior: stop distribution and ordering follow the DEBUG-trap
  contract (§4); inspection reads the same dynamic views scripts
  read (`FUNCNAME`, `BASH_LINENO`, `PIPESTATUS`, …), so the debugger
  can never disagree with the debuggee about its own state.
- Zero cost when off (one atomic load + one branch per seam, §4.4),
  asserted by a perfbudget ratchet.
- No semantic shims: the debugger fixes nothing by simulating bashdb;
  where bashdb's behavior rests on a wrong path, the design says so
  and diverges on purpose (§10 notes each case).

**Non-goals**

- Not a Rust-source debugger (that is LLDB's job, per repo practice);
  the debuggee is the *script*.
- No cross-process attach in M0/M1: background jobs and
  procsub bodies are separate OS processes (§6.3); steering them
  needs an attach protocol that is designed, not assumed (§12, P-4).
- No write-interception watchpoints until the variable-write funnel
  completes (§5.4, §12 P-3). No record/replay or reverse execution.
- No remote transport: the DAP frontend is stdio-only in M0
  (draft of record §7); no TCP, no attaching a foreign process.
- The external bashdb workflow is not removed or deprecated by this
  work; `docs/bashdb-debugging-rubash.md` stays accurate, and failing
  bashdb commands keep driving engine root-cause fixes as today.

## 2. Flag semantics

| Flag | Owner | Meaning |
| --- | --- | --- |
| `--debug` | native debugger | A GNU option — a legacy one (`shell.c:255` sets the vestigial `debugging` variable; no observable GNU behavior conflicts with the name) — adopted for the native debugger by the owner ruling of **2026-10-23**, which supersedes the 2026-10-18 DP2 answer on this point. One of the two first-class spellings of the one native debugger (see the rule below). Today rubash arms extdebug from the first command under this flag (documented niu extension, `src/invocation.rs:396-405`); the native debugger takes that meaning over, as the parser's own comment anticipates (`src/invocation.rs:23-27`). `rubash --debug script.sh` = the interactive session on the terminal (the M1 view); the owner expects this to be the spelling end users actually type (confirmed 2026-10-25). |
| `--dap` | native debugger | New flag, native citizen (2026-10-23 ruling). The other first-class spelling of the same debugger: it selects the DAP view over stdio explicitly (§13) and is the **M0 frontend** (DP1, ruled 2026-10-18) — the form editor extensions launch (`debugAdapterExecutable` → `rubash --debug --dap`, draft of record §10). |
| `--debugger` | bashdb / GNU | Unchanged: `shell.c:257` `debugging_mode`; startup runs the bashdb starter file (`src/main.rs:1532 start_debugger`), mode cleared if the starter is missing. Never repurposed (2026-10-23 ruling; the surviving half of the 2026-10-18 DP2 answer). |

Rules: **one parse meaning** (2026-10-23 ruling) — `--debug` and
`--dap` both name the native debugger, so they compose rather than
conflict: `--debug` opens the session's interactive view, `--dap`
opens the protocol view, and `rubash --debug --dap` is the
spelled-out DAP host form the editor extensions launch. One
invocation opens one view; since both views are the same session's
operations (§3), the choice of spelling is never a capability
choice. For the interactive view, `--debug` with no TTY on stdin
still runs — the command stream falls back to the controlling
terminal (§9.5); if no terminal exists either, the session refuses
to start interactively and says why (batch driving is
`source`-file based, §10). The debugger implies the extdebug *gate
predicate* internally (§4) but does not flip the user's `extdebug`
shopt observably: `shopt -p extdebug` in the debuggee reports the
user's setting, not the debugger's plumbing.

## 3. Architecture in one page

```
rubash --debug script.sh  /  editor-launched rubash --debug --dap
  ├─ invocation (existing) ── builds Executor (existing init)
  ├─ debugger session (new, src/debugger/)
  │    ├─ breakpoint table, watch table, display list, step arming
  │    ├─ stop loop: parks the executor thread at a seam, serves
  │    │   requests, resumes with an arming decision
  │    └─ event stream (DebugEvent) — engine-owned, typed
  ├─ frontends (new; views only, no semantics):
  │    ├─ DAP adapter (M0, §13): Content-Length JSON over stdio
  │    └─ CLI (M1, immediately after): gdb-grammar parser/dispatch
  │         over the engine's bind/readline surface; reads /dev/tty
  │         (§9.5)
  └─ engine (existing): executes the script; the funnel consult
       (§4.1) asks the session "stop?" at each seam
```

The session is the only new semantic owner. Each frontend
translates its input into session operations (`StepIn`,
`SetBreakpoint`, `EvaluateForPrint`, …) and renders session state
back — the CLI parses text, the DAP adapter parses protocol
requests; neither contains stop logic, frame logic, or variable
logic. The adapter is built first (DP1, ruled 2026-10-18) and the
CLI follows immediately on the same operations; any behavior that
exists in only one frontend is a bug by definition.

## 4. Engine substrate

### 4.1 Seams

The stop substrate is the existing DEBUG-trap distribution: eleven
`run_debug_trap` call sites funneling into
`src/executor/trap_exec.rs:970`. Verified at `1e72c7f9`:

| Stop site | GNU C anchor | rubash seam |
| --- | --- | --- |
| every simple command | `execute_cmd.c:4506` | `src/executor/ast_exec.rs:950` (gate `:902-942`) |
| `[[ ]]` / `(( ))`, `time` body, arith-for init/test/update, `case` head | `execute_cmd.c:4153/:3920/:3187/:3668` | `src/executor/compound_exec.rs:508, 1789, 1990, 2046, 2129, 3694` |
| function entry, before body | `execute_cmd.c:5383-5391` | `src/executor/function_calls.rs:674` |
| `for` head, per iteration | `execute_cmd.c:3053` | `src/executor/loop_select.rs:170` |
| pipeline element | `execute_cmd.c:4506` | `src/executor/pipeline_exec.rs:1014` |
| `select` head | `execute_cmd.c:3528` | `src/executor/select_exec.rs:77` |

`while`/`until`/`if`/`{ }`/`( )` heads never stop; components stop as
inner commands. Stops fire **before word expansion**.

This distribution is the stop *substrate*: it defines where the
two M0 stop granularities (§4.3) can land. Its completeness is a
reachability fact — it is what makes a line stop mean what GNU's
DEBUG trap means on that line — not a third granularity tier and
not an M0 promise of seam-kind-specific stop features (DP3).

The frame-transition patch point (the consensus review's D1 hole):
with extdebug, trap status 2 longjmps from inside `run_debug_trap`
(`trap.c:1310-1316`); the transition event therefore also fires at
the return site, `run_return_trap` (`src/executor/trap_exec.rs:1058`).

### 4.2 Ordering at a seam

GNU `trap.c:1263`: lineno update → `BASH_COMMAND` set → user DEBUG
trap runs in full → extdebug skip/force-return decision → **debugger
consult** (breakpoint match? step arming? watch re-check?) → stop or
execute. One dispatch channel; the debugger is a consult inside the
existing path, never a parallel hook stream, and breakpoint
admission reuses the `skips_debug_trap` predicate verbatim (#117
rule).

### 4.3 Stop granularity (DP3, ruled 2026-10-18)

M0 commits to exactly two stop granularities: **line stops** and
**function-entry stops**. That is the whole M0 granularity promise
(DP3; the ruling also permits adding data-assertion-level stops
during implementation, but they are not M0-committed). The seam
family of §4.1 is the substrate beneath both: a line stop lands
where the DEBUG trap would fire on that line, the function-entry
stop lands at the entry seam, and a line no seam can fire on is a
dead line — it answers not-verified with the reason, never silence
(D1 rule, §5.1). What M0 does *not* promise, so nobody reads it
in: seam-kind-targeted stop features (per-element pipeline stop
control, §6.4), data/assertion-level stops, and exception stops
(§9.3 — the filters land in M1, the parking option in M2). Seams
still carry kind tags internally; the tags drive stepping filters
(§6) and fixture attribution, not admission tiers.

### 4.4 Gating and state

One session `AtomicBool` checked before the `has_active_trap_action`
consult (`src/builtins/trap.rs:572`; gate at `ast_exec.rs:902-942`).
Session state unifies with `debug_trap_running` (`trap_exec.rs:974`
guard), `debug_trap_in_scope` (`trap_exec.rs:1148`) and functrace
inheritance, so trap and breakpoint semantics cannot diverge. No stop
inside a trap action re-enters the DEBUG path (the `SIG_INPROGRESS`
invariant, `trap.c:1270`); the return-site event reports the
transition instead. Async control while running (Ctrl-C, §9.4) is
serviced at the per-command boundary poll,
`run_pending_signal_traps` (`ast_exec.rs:729`).

## 5. Breakpoint model

Four breakpoint kinds, one table. Every entry carries: id, kind,
location, enabled flag, ignore count, hit count, optional condition,
optional command list, one-shot flag. `info breakpoints` renders the
table; ids are stable for the session.

### 5.1 Line breakpoints — `break [file:]line`, `tbreak`

- *Semantics*: stop before the command at that line executes, in any
  frame (matching is on file+line, not on frame — gdb/bashdb
  behavior).
- *Resolution*: at set time, walk the parsed AST once
  (`CommandNode.line`, `src/parser/nodes.rs:1252`) and intersect with
  the seam line set (§4.1). No node on the line, or no seam that can
  fire on it → rejected immediately with the reason ("no command on
  this line" / "DEBUG trap never fires on this line"). Resolution is
  metadata-only; there is no AST dispatch loop.
- *Landing point*: the funnel consult (§4.2) — the seam's live
  `CommandNode` gives (file via `current_bash_source`, line)
  directly; the hit check is a table lookup.
- *Sourced files and functions defined elsewhere*: locations are
  keyed by the file identity the engine itself reports
  (`BASH_SOURCE` stack), so a breakpoint set as `lib.sh:20` hits when
  that file's line runs, whoever sourced it. A bare `break 20` binds
  to the file of the *current* frame at set time (bashdb rule) and
  the binding is shown in `info breakpoints`.
- *One-shot (`tbreak`)*: same entry, one-shot flag; deleted at first
  hit, before its command list runs.

### 5.2 Function breakpoints — `break funcname`

- *Semantics*: stop at function entry, before the body, in the
  callee's frame (`showing_function_line` position).
- *Landing point*: the function-entry seam
  (`function_calls.rs:674`) — the same site the DEBUG trap fires at
  entry today, where the entry DEBUG-trap return path
  (`ExecuteError::Return` from an entry trap action, the rubash#1162
  case) already proves the frame is fully formed and inspectable.
  Resolution uses `function_definition_locations` / `body_open_line`;
  an undefined name is accepted as a *pending* breakpoint (bashdb
  errors; gdb asks) — design choice: accept and mark pending,
  resolving if the function is defined later (scripts define
  functions as they run; erroring would make `b main_helper` before
  its definition line a footgun). Pending state is visible in
  `info breakpoints`.
- Function breakpoints compose with conditions and ignore counts
  exactly like line breakpoints.

### 5.3 Conditional breakpoints — `condition id expr`, `break … if expr`

- *Semantics*: at a would-be hit, evaluate `expr` in the stopped
  frame; stop only if its exit status is 0 (bashdb/gdb rule — the
  condition is a shell command, conventionally a `[[ ... ]]` or
  `(( ... ))` test).
- *Landing point*: the funnel consult, after location match, before
  the stop is committed. Evaluation runs through the engine's own
  expansion/execution in the current frame — the same path `eval`
  uses (§7.4) — with the DEBUG path suppressed (a condition must not
  recursively fire breakpoints; the no-recursion guard of §4.4
  covers the evaluation).
- *Power rule* (applies here, to `print`/`eval`, and to command
  lists): **explicit user artifacts are full-power** — a condition
  the user typed may contain command substitution and may have side
  effects, exactly as in bashdb; the debugger does not sandbox the
  user's own commands against their own script. **Implicit
  evaluations are strict** — anything the debugger evaluates on its
  own initiative (display lists, watch re-checks, hover-style
  rendering) goes through the §7.5 AST whitelist. This rule, not a
  per-command exception list, is what keeps the safety story
  coherent.
- A condition that errors (nonzero non-test status, parse failure)
  is treated as false, and the failure is reported once per
  breakpoint (not per hit) to avoid log spam; the hit count still
  increments.

### 5.4 Watchpoints — `watch expr`, `watche expr`

GNU defines no watchpoint semantics and this engine (like every
shell) has no read interception and an incomplete write funnel, so
the design is stop-and-diff, labeled honestly:

- *Semantics*: a watch registers an expression (a variable name for
  `watch`, an arbitrary strict-subset expression for `watche`). At
  **every** stop, each registered watch is re-evaluated; if the value
  differs from the value at the previous stop, execution stops
  (or, if already stopped for another reason, the change is
  announced in the stop banner: `watch: $x 'a' → 'b'`). This is
  bashdb's model exactly — change is detected at stop granularity,
  not at the writing command; the banner says "changed since last
  stop", never "written by line N".
- *Landing point*: the stop loop, not the write path. Cost is
  O(watches) per stop, zero per command when no watch is set, and
  zero when running free (no stops → no checks → a watch alone never
  slows a run between breakpoints; this matches bashdb and is the
  documented limit of the model).
- *Cheaper diff for tracked names*: for names tracked by the
  VarTable freshness protocol (`src/shell/var_table.rs` —
  `tracked`/`stale`/`mirror_epoch`/`synced_epoch`), the stop loop
  compares epochs instead of serialized values: precise, and free
  of rendering cost. Untracked names snapshot their rendered value.
- *Not offered*: `awatch`/`rwatch` (read watchpoints) — no engine
  anywhere in this design has a read choke point, and bashdb does
  not offer them either. True write-interception watchpoints
  (stop *at* the writing command, with the writer's frame) are the
  endgame of the funnel lane (§12 P-3) and are not promised by any
  milestone in §11 before it.

### 5.5 Breakpoint administration

- `ignore id n`: the next n hits pass silently (hit count still
  counts). Evaluated at the consult, before conditions.
- `disable`/`enable`/`delete`/`clear`: table operations; `clear`
  with no argument clears the current line's breakpoints (bashdb
  rule). All take effect at the next consult — including from a
  breakpoint command list at the current stop.
- `commands id … end`: a command list run at each hit; if the list
  does not end in an explicit resume command, the session stops
  after running it (gdb rule). `silent` as the first line suppresses
  the stop banner. Command lists are explicit user artifacts
  (full-power, §5.3) and run with the DEBUG path suppressed.
- Hit counts and the `info breakpoints` columns (Num/Type/Disposition/
  Enabled/What/Ignore/Condition) mirror bashdb's rendering so the
  oracle transcripts (the consensus D6 model, §8 — DAP transcripts
  in M0, CLI transcripts at M1) can diff them mechanically.

## 6. Stepping semantics

Stepping is arming: a resume decision installs a one-shot rule at the
funnel — *stop at the next admitted seam satisfying P* — where P is
what distinguishes the commands. All definitions are in terms of
the engine's real execution structure; where that structure differs
from bashdb's mental model, the difference is stated, not smoothed
over.

### 6.1 The primitive commands

- **`step` (`s`)** — stop at the next admitted seam, whatever frame
  it is in: stepping *into* a function call stops at the
  function-entry seam and then at the body's first command.
  `step n` repeats.
  `step+` — bashdb's "step, skipping functions you don't care about"
  is not its meaning; bashdb `step+` = step but do not stop inside
  *sourced files/functions marked via `set` trace levels*… the
  portable definition this design adopts: `step+` stops at the next
  seam **in the current file**, `step-` at the next seam **leaving
  the current function** (i.e. step-out-one-level-then-stop). Both
  are P-variants (file/frame filters on the next seam), cost nothing
  extra, and are documented as rubash's reading where bashdb's own
  is tied to its internals.
- **`next` (`n`)** — stop at the next seam whose frame depth is ≤
  the depth at resume: function calls run to completion *as seen
  from this frame*. Implementation: P = (depth ≤ resume depth) at
  the line and function-entry seams; the callee's seams are
  admitted by the funnel but fail P. Note this makes `next` over a long-running callee exactly
  as blocking as bashdb's — there is no backgrounding of shell
  execution.
- **`finish`** — run until the current frame returns: arm the
  return-site emission (§4.1) for the current frame identity, plus
  P = (depth < resume depth) as a backstop (covers the
  extdebug-skip longjmp path, where the return event is the
  *only* signal). At the main frame, `finish` = run to script end
  (the EXIT path, §9.3). On arrival, print the frame's return status
  (`$?` as the frame leaves it) — gdb's "Value returned is $1"
  analogue is the frame's exit status and `FUNCNAME` pop, both read
  from engine state.
- **`return [status]`** — force the current function to return now:
  the engine already has this exact path — a `return` raised inside
  the entry DEBUG-trap action is consumed by the normal result path
  (see the rubash#1162 handling at the entry seam). The debugger
  raises the same `ExecuteError::Return` from the stop, with the
  given status; at the main frame `return` is refused (there is no
  caller; bashdb exits the sourced file instead — for the main
  script, refuse with that explanation).
- **`until`** (gdb; bashdb lacks it) — stop at the next seam in the
  current frame whose line is *greater* than the current line: the
  loop-escape step. Cheap (P adds a line comparison), genuinely
  useful in loops, included in the completeness phase (§11) rather
  than the minimal one, and labeled a rubash addition, not a bashdb
  parity item.
- **`skip [n]`** — do not execute the next command; resume at the
  seam after it. The engine owns this mechanism already: it is the
  extdebug skip decision (§4.2) with the debugger as the decider
  instead of a trap's exit status. `skip` therefore arms "consult
  returns Skip" for exactly one seam. Skipping a compound command
  skips the whole construct where the construct has a head seam
  (the seam *is* the head for `case`, `for`, `select`, `[[ ]]`,
  `(( ))`, arith-for); for constructs with no head seam
  (`while`/`until`/`if` heads never stop, §4.1) `skip` applies to
  the first inner command and says so.
- **`continue` (`c`)** — resume with no arming; run to the next
  breakpoint/watch/terminating event.

### 6.2 Stepping and the frame stack

Frame depth for P is the engine's function-call depth
(`function_name_stack` length at the seam), not a debugger-side
counter — a debugger counter would desync on exactly the paths that
matter (extdebug skips, `return` from trap actions, error unwinds).
Synthesized frames (subshell/comsub, §6.3) do not increment the
function depth; `next` from a frame whose next command *contains* a
comsub runs the comsub to completion (it is part of the command),
while `step` stops at the comsub's first inner command *if* the
comsub executes in a linked child executor (§6.3) — and the stop
banner names the synthesized frame (`$(...) depth 1`).

### 6.3 Subshells, command substitution, pipelines — the materialization model

This engine does not fork for synchronous constructs; it
**materializes**:

- *Subshells `( ... )` and comsub `$( ... )`* run in snapshotted
  child executors (`Executor::new` + state snapshot import — cf.
  `compound_exec.rs:3220` on the child's construction), in-process,
  sequentially. Their variable writes live and die in the snapshot
  (GNU parity: subshell writes never propagate).
- *Pipelines* are driven by a sequential loop over stages with
  materialized, *timed* stdin (`pipeline_exec.rs:870-1060`,
  `timed_stdin`) for shell stages; purely external stages spawn real
  processes. The pipeline-element seam (`pipeline_exec.rs:1014`)
  fires per stage in the driving executor.
- *Background jobs* are the exception: they re-exec rubash as new
  OS processes with traps stripped. *Process substitution* bodies
  likewise run in spawned same-exe children
  (`trap_exec.rs:2877 bind_input_process_substitution_child`,
  `external_setup.rs:915`).

Consequences for stepping, by construct:

| Construct | `step` into it | Inspection while inside | Degradation |
| --- | --- | --- | --- |
| function call | yes — entry seam | full (real frame) | none |
| subshell `( )` | yes — synthesized frame on the same stack | reads/writes hit the *snapshot*; banner marks the frame `subshell:` | write-back (§7.6) affects only the snapshot — GNU parity, stated in the banner |
| comsub `$( )` inside a word | yes, when the comsub is on the command being stepped into; the stop is inside the expansion of that command, before the outer command executes | snapshot state | a comsub inside a *breakpoint-less* free run is never stopped in — same as functions |
| pipeline stage (shell stage) | the element seam fires per stage; `step` walks stages in drive order | the driving executor's state; stage-local state is the stage's snapshot where the engine snapshots | external stages are opaque: no stops inside `grep` — the seam fires for the stage as a unit, which is also bashdb's effective behavior |
| background job `&` | **no** — separate process, no channel | `info` shows the job table entry only | full degradation until the attach protocol (§12 P-4); the debugger says so at the `&` seam instead of silently stepping over |
| procsub `<( )` body | **no** — spawned child process | none before the attach work (M3+) | full degradation; additionally its diagnostics' line numbers await the #1184 f4 seeding (§12 P-1) |

### 6.4 What "in a pipeline" means for breakpoints

A line breakpoint on a line that is a pipeline fires once per
execution of the line (at the simple-command seam for the line's
first stage context). Per-element stops ride the pipeline-element
seam when *stepping* (§6.3); they are not a separate breakpoint
granularity in M0 (§4.3). This matches the DEBUG trap's own
distribution, which is the fidelity anchor: any behavior bashdb
exhibits here comes from the same distribution, so parity is
structural, not tuned.

## 7. Inspection surface

Everything in this section is a *rendering of state the engine
already maintains*. The debugger introduces no shadow bookkeeping
for frames, variables, or jobs; if a view below cannot be built from
existing state, that is a prerequisite item (§12), not a debugger
feature.

### 7.1 Stack and frames — `where`/`bt`, `frame`/`up`/`down`

- The stack is a zip of the engine's dynamic views:
  `funcname_stack` (`src/executor/dynamic_arrays.rs:1062`),
  `current_bash_source` (`:1066`), `bash_lineno_view` (`:779`), with
  the script-main-frame rule already settled
  (`has_script_main_frame`, `:700`). The `caller` builtin reads the
  same stacks, so `where` and `caller 0` cannot disagree.
- Frame 0 is the innermost executing frame; synthesized frames
  (subshell/comsub, §6.3) are interleaved and labeled. Frame
  *selection* (`frame n`, `up`, `down`) changes only which frame
  inspection commands default to — execution always resumes from
  the real innermost frame, and the prompt shows the selected frame
  number (bashdb convention). Selecting a synthesized frame shows
  its snapshot state (§6.3) and says so.
- `info args` renders the selected frame's positional parameters
  from the same per-frame storage `BASH_ARGC`/`BASH_ARGV` render
  from.

### 7.2 Variables and scopes — `info variables`, `info locals`, `print`

- Reads resolve exactly as the debuggee resolves them: innermost
  `local` scope outward to globals, through the engine's own
  lookup — never a raw map index. A name that is declared-but-unset
  renders as such (distinct from unset, distinct from empty), using
  the VarTable attribute bits (`src/shell/var_table.rs`: the ten
  declare-family attributes are authoritative there).
- `info variables` partitions by attribute (locals / globals /
  arrays / exported / read-only), reusing the renderers behind
  `declare -p` and `typeset -p` so formats match what the user
  already knows; `__RUBASH_*` internals are filtered by the same
  filter child-env construction uses (one filter, two consumers).
- Values render with the engine's quoting (`${x@Q}`-equivalent) when
  they contain control characters or carrier bytes — a debugger that
  prints a raw U+E4xx sentinel has leaked an implementation detail;
  rendering goes through the same dequoting path as `declare -p`.

### 7.3 Arrays and dynamic state — `PIPESTATUS`, `$?`, jobs

- Indexed and associative arrays render as expandable children
  (element count, per-element lines, sparse indices shown
  explicitly — a sparse array must not render as if dense).
  Rendering reads the typed array storage, not a re-parse of
  `declare -p` text.
- `PIPESTATUS` is a first-class inspection target, not an accident
  of it being an array: it is the dynamic view the engine rebuilds
  at each pipeline boundary (`ast_exec.rs:1974`, GNU
  `execute_cmd.c:1210-1224`), plus its materialized-cell form
  (`declare_local.rs:1034-1042`). At a stop *before* a command,
  `PIPESTATUS` shows the **previous** pipeline's statuses — the
  stop banner does not display it by default (it would invite the
  classic misreading), but `print $PIPESTATUS` / `info variables`
  show the true value, and the design records the semantics here so
  the behavior is a decision, not a surprise.
- `$?`, `$$`, `$!`, `$LINENO`, `$BASH_COMMAND` and friends render
  through the same dynamic views the expander uses. `$BASH_COMMAND`
  at a stop is the about-to-run command (the §4.2 ordering sets it
  before the consult), which is also what the stop banner prints.
- Jobs: `info program`-adjacent job rendering reads the engine's
  job table; a job whose completion notification is pending shows
  as such. (The in-process job-notification regression tracked as
  part_036 is a §12 P-2 prerequisite precisely because the debugger
  must not render a job state the engine itself gets wrong.)

### 7.4 Explicit evaluation — `print`, `examine`, `eval`, `shell`

- `print expr` evaluates one expansion in the selected frame and
  renders the result (value + the `$`-history slot is not
  maintained; bashdb's `$_` / gdb value history is a deliberate
  omission, §10). Full-power (§5.3's rule): the user typed it.
- `examine expr` is `print` with format control (the bashdb
  `x/`-style rendering: as string / as array / quoted / with
  attributes) — a rendering choice, not an evaluation difference.
- `eval cmd...` executes arbitrary commands in the selected frame
  through the engine's normal parse+execute, with the DEBUG path
  suppressed for the duration (an `eval` that stops at its own
  first command is useless; a nested stop requires the explicit
  `debug` command, §10). Side effects are the user's — this is the
  documented bashdb contract.
- `shell cmd...` runs an OS command in a *child* executor (not the
  debuggee's frame): the distinction bashdb draws between `eval`
  (debuggee context) and `shell` (outside it) is preserved by
  running `shell` in a snapshot child whose writes cannot leak
  back (§6.3's model, used deliberately).

### 7.5 Implicit evaluation — the strict whitelist

Anything the debugger evaluates *without* a fresh user command —
`display` lists (§10), watch re-checks (§5.4), condition-less
banner annotations — goes through the consensus-D4 checker: parse
with the engine's own parser, inspect the AST, never regex. Allowed:
plain/positional parameters, array subscripts, `${!x}` indirection,
casemod, `${x@Q}`, pattern/substring operations, assignment-free
arithmetic. Rejected (the display/watch is skipped and marked, not
executed): `$(...)`, backticks, procsubs, `${x:=y}` and `${x=y}`,
`${x:?}` and `${x?}`, `$((x+=1))`, `$((x++))`, `x--`. This is the
single place the whitelist applies; §5.3's rule keeps it from
creeping into explicit commands.

### 7.6 Write-back — `setVariable`'s CLI form: assignments, `set args`

- **Allowed, as explicit commands only.** `eval x=5`,
  `eval arr[2]=v`, `eval local y=1` (inside a function frame) all
  work, because they *are* assignments: they execute through the
  engine's real assignment path in the selected frame, so typed
  cells, attributes (`declare` flags the user includes), nameref
  resolution, and the VarTable freshness protocol
  (`note_mirror_write` choke, epochs) all stay coherent **by
  construction** — there is no debugger-private write path to keep
  in sync. This is the write-back design in one sentence: the
  debugger never writes a variable except by running the
  assignment the user typed, through the machinery every other
  assignment uses.
- A convenience form (`set var x = 5`-style, if the grammar admits
  it) desugars to the same path; it is not a second mechanism.
- `set args` rewrites the selected frame's positional parameters
  through the same frame storage `shift`/`set --` use.
- **Not allowed**: writing read-only variables (the real path
  refuses, and the refusal is the answer — no debugger override),
  writing `PIPESTATUS`/`FUNCNAME` and the other dynamic views
  (they have no storage to write; the assignment path already
  treats several as noassign, cf. `array_assignment_exec.rs:305-310`),
  and any write from an *implicit* evaluation (§7.5 rejects the
  forms that could write).
- Write-back into a **synthesized frame** (§6.3) writes the
  snapshot: when the subshell/comsub ends, the write dies with it.
  GNU parity — but the banner marks synthesized frames precisely so
  this never surprises.

## 8. Acceptance model

The consensus-D6 model, in the draft of record's form for M0 and
extended to the CLI at M1. **M0**: every fixture = script + a
recorded framed DAP transcript + the bashdb oracle log, under
`target/issue-suites/results/dap/`. The M0 minimum golden set (D6,
as pinned by the draft of record §8): (1) initialize/handshake +
capabilities; (2) `launch` → `configurationDone` → `terminated`
lifecycle; (3) `setBreakpoints`: hit + dead-line `verified: false`;
(4) `next`/`stepIn`/`stepOut` across a function; (5) `stackTrace`
cross-checked against bashdb stop positions and
`FUNCNAME`/`BASH_LINENO`; (6) `scopes`/`variables` incl. array
children; (7) `evaluate`: safe form + rejected `:=`/`$(...)`
forms; (8) `output` categories (stdout/stderr/console). Plus the
three C-level pins from the consensus review: RETURN trap
suppressed when function entry is skipped
(`execute_cmd.c:5391`); `line_number` restored on skipped
case/arith (`execute_cmd.c:3670-3675`); `BASH_COMMAND` =
`the_printed_command_except_trap` during trap bodies
(`execute_cmd.c:4502-4505`).

**M1**: the same scenarios re-rendered as CLI transcripts —
fixture = script + a command file + the recorded session
transcript + the bashdb oracle transcript for the same script and
command sequence. The oracle is *comparative*, not normative:
where bashdb and the C source disagree, the C source wins and the
fixture records the disagreement (§10's deliberate divergences are
the pre-registered cases). Minimum CLI fixture families mirror
the §10 rows marked M1: breakpoint hit/pending/dead-line,
function entry, step/next/finish across a function,
return-with-status, frame inspection incl. arrays and
`PIPESTATUS`, eval/print + rejected implicit forms, jobs
rendering, restart/kill/quit lifecycle. Timing-sensitive fixtures
(signal stops) are quarantined from the byte-diff set and asserted
structurally.

## 9. Interactions

### 9.1 The user's DEBUG trap

The debugger does not consume, replace, or suppress the user's
DEBUG trap. Ordering is §4.2's: the trap runs in full at every seam,
*before* the consult; a session with both a DEBUG trap and
breakpoints sees trap effects first, then the stop. Practical
consequences, all deliberate:

- A DEBUG trap that itself sets variables the breakpoint condition
  reads is well-defined (trap first) — and documented, because
  bashdb users have the same ordering and scripts in the wild
  depend on it.
- `set -o functrace` inheritance is shared state (§4.4): the
  debugger neither forces nor clears it; function-entry stops in
  inherited contexts follow the same `debug_trap_in_scope` answer
  the trap would get.
- A DEBUG trap action that executes `return` at the function-entry
  seam (the rubash#1162 path) still force-returns the function; if
  a function breakpoint was also armed there, the frame is gone
  before the stop could park — the session emits the return-site
  event (§4.1) so the user sees *why* the entry stop never parked.
- Breakpoint command lists and conditions run with the DEBUG path
  suppressed (§5.3/§5.5); the user's trap does **not** fire for
  debugger-initiated evaluations. (If it did, a `display` would run
  the trap per stop — a recursion and a lie about what executed.)

### 9.2 The RETURN trap

- `finish` and natural returns interleave with a user RETURN trap
  thus: the frame's body completes → the return-site event (§4.1)
  and the RETURN trap both belong to the unwind; the trap runs as
  part of the frame's execution (it can inspect `$?` and the
  frame's locals before they dissolve), and the debugger's
  `finish` stop parks *after* the trap, in the caller, with the
  frame already popped — matching what the user observes from
  `caller`-level state. The C pin from the consensus holds: when
  function entry was skipped (extdebug), the RETURN trap is
  suppressed (`execute_cmd.c:5391`), and the debugger's model of
  the frame stack follows the same suppression rather than
  inventing a frame to stop in.
- `return [status]` from the debugger (§6.1) raises the same path
  as a script `return`, so the RETURN trap **does** fire for it —
  the user is simulating a return, and the simulation is faithful
  or it is nothing.

### 9.3 The ERR trap, `set -e`, and the EXIT trap

- The ERR trap runs after a command completes nonzero
  (`maybe_run_error_trap`, `trap_exec.rs:1624`), under errtrace
  inheritance rules. Debugger stops are *before* commands, so the
  two never contend for the same instant; the interaction is
  sequential and observable: step over a failing command and the
  ERR trap (if armed and inherited) runs before the next stop, and
  its effects are visible at that stop.
- **Exception stops** ("stop when the ERR trap would fire / when
  `set -e` would exit") are a *notification at the ERR site*, not a
  veto: the filters and `exceptionInfo` that name the site land in
  M1 (draft of record §7/§10); the session may additionally park at
  the ERR-trap site before the trap action runs (a session option,
  M2), but it cannot cancel errexit. If `set -e` decides the
  shell exits, it exits; the debugger's job is to park *before*
  the exit path runs (the same site) so the user can inspect the
  corpse. This is stated plainly because every shell debugger
  user eventually asks "why didn't it stop before dying" — the
  answer is designed, not accidental.
- Session end: `quit` (and EOF on the command stream, and the
  debuggee finishing) runs the debuggee's EXIT trap and exit path
  exactly as an un-debugged run would, unless the user issues
  `kill` (§10), which terminates the session without the exit
  path and reports the debuggee as killed. `continue` to
  completion is indistinguishable from a normal run, traps
  included.

### 9.4 Signals

- **Ctrl-C at the terminal while the debuggee runs**: SIGINT is
  delivered to the foreground process group, i.e. to rubash
  itself. The session's contract: if a debugger session is active,
  SIGINT means *stop at the next seam* (the stop flag is serviced
  at the boundary poll, §4.4) and the debuggee's own SIGINT trap,
  if any, still gets its delivery through the engine's normal
  signal path — the debugger does not eat the signal; it adds a
  stop. Order at the seam: pending signal traps dispatch at the
  boundary (existing engine behavior), then the debugger stop
  parks. If no session is active, nothing changes (the flag is
  off; §4.4's zero-cost rule).
- **Ctrl-C at the debugger prompt**: interrupts the *input line*
  (readline's job, via the bind surface), never signals the
  parked debuggee — it is not running.
- **Signals sent to the debuggee** (`kill` from another terminal,
  or the debugger's own `signal` command, §10): they queue through
  the engine's self-signal/kernel backends and dispatch at the
  next boundary as usual; if a trap is installed it runs there,
  and a debugger stop armed for the same boundary parks after the
  trap (§9.1's ordering, applied to signals). An untrapped fatal
  signal kills the debuggee; the session reports the death
  (128+sig status) and offers `restart`/`quit` — it does not
  pretend the process can be resumed.
- **SIGCHLD / job notifications while parked**: notifications are
  computed at boundaries; while the executor is parked in the stop
  loop no boundary passes, so no notification prints mid-stop —
  they flush on resume, in order. (This is why part_036 is a
  prerequisite, §12 P-2: the flush must be the engine's correct
  one.)
- The whole section assumes the signal-trap coverage work
  (rubash#1511) has landed: a debugger built on a trap layer that
  silently drops deliveries would inherit the drops. That
  dependency is §12 P-2, a hard prerequisite for the signal rows
  of §10 (M2), not for the M0/M1 core.

### 9.5 Terminal and fd discipline

- The debugger's command stream is **never the debuggee's stdin**.
  The REPL opens the controlling terminal (`/dev/tty`; on Windows,
  the console device) for reading at session start; if the launch
  already has a TTY as stdin it may use it, but a script doing
  `read x` must never consume a debugger command, and a debugger
  `eval read x` must read from the *script's* stdin. This is the
  single most important fd rule in the design (it is what
  bashdb's `tty` command exists to manage).
- Debuggee stdio is untouched: its stdout/stderr flow where the
  launch put them; debugger output goes to the terminal (or, when
  the terminal is the same stream, interleaves — with the stop
  banner printed to stderr so a piped stdout stays parseable).
- `source` command files (§10) come from files, not the terminal,
  and are how batch/golden sessions drive the CLI (§8).

### 9.6 Lineno accounting

- The debugger's line numbers *are* the engine's: the same
  accounting that feeds `BASH_LINENO`/`LINENO` (#1336 family) and
  the frame views (§7.1). Where that accounting is wrong, the
  debugger is wrong in exactly the same way the script is — one
  ledger, fixed at the root, never patched over in the debugger.
- Sourced files: each file's lines are its own; breakpoints and
  listings key on the engine's file identity (§5.1).
- `eval`'d and condition/command-list code has no file line; it
  renders under the pseudo-source `<debugger>` with its own
  internal line numbers, and breakpoints cannot target it.
- Procsub bodies: separate processes with (until §12 P-1 lands)
  line-1 diagnostics; the debugger neither stops inside them
  (§6.3) nor fabricates lines for their frames.

## 10. Coverage against bashdb's command surface

Verdicts grade the **CLI command surface**: **M1** = in the CLI
phase (the minimal usable interactive debugger, landing
immediately after M0); **M2** = in the completeness phase;
**M3+** = endgame; **No** = deliberately not offered, with the
reason. The DAP surface's own phasing is §13. The bashdb column is
its documented command set (bashdb 5.0 manual); "parity" means same
user-visible semantics, not same internals.

| bashdb command | Verdict | Notes |
| --- | --- | --- |
| `help` | M1 | Command help is part of the grammar from day one; undocumented commands in a debugger are how users lose trust. |
| `break` (line / function / `file:line`) | M1 | §5.1/§5.2. Bashdb's `+offset` relative forms are M2 (rare, and the resolution rules are fiddly). |
| `tbreak` | M1 | One-shot flag on the same entry; no new machinery. |
| `condition` | M2 | §5.3. Needs the consult-side evaluation wiring; M1 breakpoints are unconditional. |
| `ignore` (hit counts) | M2 | Rides with conditions (same consult path). |
| `commands` (breakpoint command lists) | M2 | §5.5. Needs the suppressed-DEBUG command runner + `silent` semantics. |
| `enable` / `disable` / `delete` / `clear` | M1 | Pure table operations. |
| `watch` / `watche` | M2 | Stop-and-diff model (§5.4), labeled honestly. True interception is M3 (§12 P-3). |
| `awatch` / `rwatch` | No | gdb vocabulary bashdb itself does not offer; no read choke point exists to build them on. |
| `step` / `step+` / `step-` | M1 | §6.1, with rubash's reading of `+`/`-` documented where bashdb's is internal. |
| `next` | M1 | §6.1. |
| `skip` | M2 | The engine owns the skip mechanism (extdebug decision); wiring the debugger as its decider is M2 (§6.1). |
| `finish` | M1 | §6.1 via the return-site event. |
| `return [status]` | M1 | Raises the engine's existing Return path (§6.1). |
| `until` | M2 | gdb command bashdb lacks; cheap P-variant, labeled a rubash addition (§6.1). |
| `continue` | M1 | — |
| `quit` / `exit` | M1 | Runs the debuggee's exit path (§9.3). |
| `kill` | M1 | Terminates the session without the exit path; status reported as killed. |
| `restart` / `run` | M2 | Restart = re-exec the host with the original argv (a clean process), not an in-process reset — an in-process "restart" would have to un-run every side effect the debuggee already committed, which no shell can do honestly. |
| `where` / `backtrace` | M1 | §7.1. |
| `frame` / `up` / `down` | M1 | Selection for inspection only (§7.1). |
| `list` | M1 | Source text from the frame's file; function bodies from their definition locations. Listing is the feature users touch most after `print`; it is not deferrable. |
| `search` / `reverse` (`/`, `?`) | M2 | Text search over the frame's source file; useful, not core. |
| `print` | M1 | §7.4. No gdb value history (`$1`, `$_`) — shell users have the variables themselves; a parallel value-history namespace is state for state's sake. |
| `examine` | M2 | Format-controlled rendering (§7.4). |
| `display` / `undisplay` | M2 | Implicit evaluation → strict whitelist (§7.5) applies. |
| `eval` | M1 | Full-power explicit evaluation in the selected frame (§7.4). |
| `shell` | M2 | Child-executor OS commands (§7.4). |
| `info breakpoints` | M1 | Renders the §5 table. |
| `info locals` / `info variables` | M1 | §7.2. |
| `info args` | M1 | §7.1. |
| `info functions` | M1 | Function table + definition locations (the same data function breakpoints resolve through). |
| `info files` / `info source` | M1 | Files the session knows (main, sourced), with their breakpoint counts. |
| `info program` | M2 | Run state, last stop reason, job table (§7.3). |
| `show` / `set` (debugger settings) | M1 subset → M2 | M1: `listsize`, prompt basics. M2: `linetrace` (§trace), `logging` (session transcript to file — also the §8 fixture mechanism), `basename`, `showcommand`, `args` (debuggee positional rewrite, §7.6). |
| `set linetrace` / `trace` / `untrace` | M2 | Trace mode = the seams with a sink instead of a park: every admitted command prints `PS4`-style before running, no stopping. Rides the funnel; the cost is a sink branch. |
| `handle` / `signal` | M2 | Signal disposition for the debuggee (pass/stop/print) and delivering a signal — after §12 P-2 lands; semantics in §9.4. |
| `tty` | M1 (report) / M2 (set) | Reporting which stream the debugger owns is part of the fd contract (§9.5); rebinding it mid-session is M2. |
| `pwd` / `cd` | M2 | The debugger shares the debuggee's cwd (one process); `cd` therefore changes the *debuggee's* cwd and is labeled as such. |
| `source` (command file) | M2 | Also the batch driver for golden sessions (§8). |
| `history` (`H`, `!`) | M1 | Inherited from the bind/readline surface the CLI is built on. |
| `complete` | M1 | Command/address completion through the same bind machinery; the completion table is small because the grammar is fixed. |
| `alias` / `unalias` (debugger-local) | M2 | Debugger aliases live in the session, **never** in the debuggee's alias table — polluting the debuggee is the kind of side effect that makes debuggers lie. |
| `edit` | M3+ | Opens `$EDITOR` at the current line. Trivial mechanics, but it suspends the session's terminal discipline; defer until the CLI is boring. |
| `file` (associate script) | No | Launch already binds the debuggee; bashdb needs `file` because it can start without one. A session that starts without a script is a different product (an interactive shell with a debugger on call) — see `debug` below. |
| `load` | No | bashdb loads *debugger helper scripts* — an artifact of bashdb being a script that extends itself. The native debugger's extension point is the command file (`source`) and the session API both frontends expose; there is no second script runtime to load into. |
| `export` (debugger vars → debuggee env) | No | Same artifact class as `load`: session state is engine-side and typed; there are no debugger shell variables in the debuggee to export. Anything a user wants in the debuggee is one `eval export …` away. |
| `set annotate` | No | A frontend-protocol legacy level for 1990s GUD buffers; the native frontends are the DAP adapter and the CLI. |
| `set debugger` (debug the debugger) | No | A bashdb self-hosting artifact. The native debugger is Rust: its debugging story is the repo's normal one (LLDB on the engine), not a session mode. |
| `debug` (start debugging a command/script from inside a running shell) | M3+ | The genuinely interesting bashdb feature this table defers: entering the debugger from a live session. It needs the session to be installable mid-flight (breakpoint table seeded against an already-parsed world) — the machinery is §3's, but the lifecycle design is its own item, and it composes with the dynamic-activation idea (`_Dbg_debugger` in bashdb; a `breakpoint`-style builtin here) rather than being bolted on. |
| `_Dbg_debugger` (script-side activation) | M3+ | With `debug` above: a script that can break into the debugger at a chosen line. Designed as a builtin that consults the session table (and is inert — a no-op returning success — when no session exists, so instrumented scripts still run unde­bugged). |

Deliberate divergences from bashdb, collected: pending function
breakpoints (§5.2), restart-by-re-exec, no value history, no
`load`/`export`/`annotate`/`set debugger`, debugger-local aliases.
Each is recorded at its row; none is discovered later by surprise.

## 11. Milestones, broken into estimable work packages

Estimates are single-engineer ranges (days), anchored to the tree:
the seam funnel is one function; the new module is `src/debugger/`;
the CLI grammar is ~25 commands in M1, ~20 more in M2. All executor
touches go through the captain-exclusive `src/executor/**` lane.
Every work package lands with its regression fixtures in the
repo's own suites (issue-numbered tests, GNU/bashdb-oracle
expectations), per the standing rule that fixes and their tests
ship together — for the debugger the "suite" is the §8 transcript
harness plus issue-numbered Rust tests for session logic.

**Prerequisites** (own lanes, §12): P-1, P-2 (for the
signal rows), P-5. None blocks M0 except as noted there.

**M0 — engine, session, and the DAP frontend (DAP-first, DP1)**

| WP | Content | Est. |
| --- | --- | --- |
| WP-1 | Event-semantics definition for the five core seam groups (simple command, function entry, function return, boundary poll, skip/longjmp path): event shapes, ordering vs traps, kind tags. The captain-review artifact — the first deliverable of M0 (draft of record §0). | 3–4 d |
| WP-2 | Session core: state machine (running/stopped/terminated), breakpoint table (§5 data model), gating flag + state unification (§4.4), `DebugEvent` types in `src/debugger/`. No frontend. | 5–7 d |
| WP-3 | Funnel consult + stop parking: session check inside `run_debug_trap`'s path after the extdebug decision, return-site emission, admission of the two M0 granularities (§4.3), async stop flag at the boundary poll. | 4–6 d |
| WP-4 | Stepping engine: arming predicates for step/next/finish/return/continue (§6.1), depth from the engine's stack, synthesized-frame labeling (§6.2/§6.3). | 4–5 d |
| WP-6 | Inspection rendering: frames (§7.1), variables/scopes (§7.2), arrays + dynamic state (§7.3) — the session-side views both frontends render (DAP `stackTrace`/`scopes`/`variables` in M0; the CLI `info` family at M1). | 5–7 d |
| WP-7 | Explicit evaluation: in-frame evaluation (DAP `evaluate`; the CLI's `print`/`eval` at M1), the §7.5 AST checker (shared with later implicit uses), write-back mechanism via the real assignment path (§7.6; its DAP `setVariable` mapping is WP-15, M1). | 5–7 d |
| WP-8 | Breakpoint resolution: AST line walk, file identity, pending function breakpoints, `verified: false` resolution reporting (DAP) and the CLI's rejection messages ("no command on this line" / "DEBUG trap never fires on this line"). | 3–4 d |
| WP-10 | Release gates in CI: flag-off perfbudget (startup/loop within noise), binary-size ratchet (≤512KB over the re-measured baseline). Lands with WP-3, not after it. | 2–3 d |
| WP-12 | DAP transport + protocol layer (§13): Content-Length framing, hand-rolled JSON subset (no serde), dispatch for the §13 request set. | 8–12 d |
| WP-13 | DAP rendering: adapter-shaped frames/variables over WP-6's views, plus the reuse audit — any rendering the adapter needs that WP-6 does not provide is a session-service gap, fixed in the session, not in the adapter. | 4–6 d |
| WP-14 | Golden DAP transcripts (§8 M0 set) wired into the repo suites; bashdb oracle logs recorded alongside, disagreements filed per fixture. | 5–7 d |

M0 total ≈ **48–68 engineer-days**, with WP-12/WP-13
parallelizable after WP-2/WP-3 freeze the session interface
(~week 3). Exit criterion: the §8 golden DAP set is green, the
release gates are green, and a multi-function script can be
driven end-to-end through the adapter (break / step / inspect /
evaluate / lifecycle) by the transcript harness — the draft of
record's acceptance bar.

**M1 — the CLI, immediately after M0** (draft of record §10):

| WP | Content | Est. |
| --- | --- | --- |
| WP-5 | CLI: grammar parser + dispatch for the M1 command set (§10 M1 rows), stop banner, prompt over the bind/readline surface, `/dev/tty` command stream (§9.5). The largest single package. | 8–12 d |
| WP-9 | CLI transcript harness + golden fixtures (§8) wired into the repo suites; bashdb oracle runs scripted, disagreements recorded per fixture. | 5–7 d |
| WP-15 | `setVariable` (the DAP mapping of §7.6) + `exceptionInfo` and the exception filters (`err-trap`, `set -e`, `parse-error`) on the existing ERR/parse plumbing (§9.3). | 3–5 d |
| WP-16 | Zero-JS VS Code extension: `debugAdapterExecutable` → `rubash --debug --dap`. | 2–3 d |
| WP-11 | Docs: user guide for `--debug`, the bashdb-divergence list (§10) as a docs page, `docs/bashdb-debugging-rubash.md` cross-reference update. | 2–3 d |

M1 total ≈ **20–30 d**. Exit criterion: a user can debug a
multi-function script end-to-end at the terminal
(break/step/inspect/eval/lifecycle) with the M1 rows of §10
behaving per spec, CLI goldens green, gates green.

**M2 — completeness**: conditions + ignore counts
(3–4 d, consult-side wiring on WP-3/WP-7); breakpoint command
lists (3–5 d); watch/watche stop-and-diff incl. epoch-diff for
tracked names (4–6 d); display/undisplay (2–3 d); `skip` +
`until` (2–3 d); extended per-seam fixtures across the full
substrate (3–5 d, fixtures only — the substrate itself is M0;
this buys stop-fixture coverage per seam kind, not new
admission); trace mode (2–3 d); exception-stop parking option
at the ERR site (3–4 d, §9.3; the filters themselves are M1);
`handle`/`signal` (2–4 d, after P-2); restart by re-exec
(2–3 d); `source`, debugger aliases, `search`, `examine`
formats, `set/show` completion, `info program` (5–7 d
combined). M2 total ≈ **33–49 d**.

**M3 — endgame**: true write-interception watchpoints on the
completed funnel and the trace ring buffer post-mortem (the
#497 record's M3; after P-3 — the debugger-side watchpoint
wiring is small, 3–5 d, the lane itself is P-3's estimate);
background-job attach protocol (design 5–8 d, implementation
its own RFC) and `debug` / script-side activation (§10) as M3+.

## 12. Prerequisite engine work (must land on its own lanes first)

| # | Item | Why the debugger needs it | Blocks |
| --- | --- | --- | --- |
| P-1 | Procsub-body lineno seeding (rubash#1184 f4): spawn of procsub bodies carries the construction line into the child. | Debugger line identity (§9.6) and any future procsub stop support; also a diagnostics correctness fix in its own right. **Landed 2026-10-25 (PR #1522).** | Nothing in M0; required before procsub frames can ever be honest. |
| P-2 | Signal-trap coverage (rubash#1511) + the part_036 in-process job-notification fix. | §9.4's contract (Ctrl-C stop, trap-then-park ordering, notification flush on resume) is only as true as the trap layer underneath; job rendering (§7.3) needs correct notifications. **Landed 2026-10-25 (PR #1527).** | The `handle`/`signal` rows and §9.4's guarantees; the M0/M1 core proceeds without it. |
| P-3 | VarTable **value-write** funnel completion: route the ~195 raw `env_vars.insert/entry` sites and the `ARITH_WRITES` log (`src/executor/arithmetic/mod.rs:25`) through choke-point methods, the way the attribute/mirror side already is (`src/shell/var_table.rs`, `note_mirror_write`). | The only path to true write-interception watchpoints (§5.4). Also independently justified: those sites are GNU's dynamic-assign set and deserve one semantic owner. Own lane, own regression ledger, never bundled into a debugger PR. | M3 watchpoints only. Explicitly **not** an M0–M2 blocker. |
| P-4 | Attach protocol design for re-exec'd processes (background jobs, procsub bodies): a control channel (pipe/socket + a session handshake) designed against the fact that these are separate rubash processes with stripped traps. | §6.3's two full degradations end only here. Design first; implementation is an M3+ RFC. | Nothing before M3. |
| P-5 | CI perfbudget + binary-size ratchet jobs (neither exists today). | §4.4's zero-cost claim and the ≤512KB gate must be asserted by CI from the first consult PR, or they will be asserted never. | WP-3 (lands together, WP-10). |
| P-6 | Placement vs the 1.5.0 core split (#595 RFC §6): the `debugger` module's crate home (`rubash-core`) and the `DebugEvent` stream's status as a core event-callback family (typed bridge, not a `__RUBASH_*` env channel). | Avoids building the module in a home it immediately leaves; the RFC already names the debugger a natural core consumer. | WP-2's module placement — **decided 2026-10-25 (§14 Q2 = (c))**: the session core is built in-tree now, shaped for its `rubash-core` home; the funnel consult lands with/after the split; the fallback (build in-tree, move wholesale) is priced and acceptable if the split slips past 1.5.x. |

## 13. The `--dap` frontend (M0)

The M0 frontend (DP1, ruled 2026-10-18), unchanged in substance
from the draft of record §§0–2: a DAP server over stdio
(Content-Length framing, hand-rolled JSON subset — the repo has
no serde dependency and adds none) translating requests into
the *same* session operations the CLI will use at M1. The
adapter holds zero semantics (§3). Both request forms of the one
supported topology are honored: `launch` spawns the script
in-process after `configurationDone`; `attach` is honored only
for the same-process spawn shape this host already owns, and is
rejected with a clear message for anything else (background jobs
are separate OS processes and are not attachable, §6.3).
Request set for M0: `initialize`, `launch`, `attach`,
`setBreakpoints`, `setExceptionBreakpoints` (accept — VS Code
sends it unconditionally; claim zero filters until the M1
filters land), `configurationDone`, `threads` (single thread +
synthesized frames, the consensus D3 model), `stackTrace`,
`scopes`, `variables`, `evaluate`, `next`, `stepIn`, `stepOut`,
`continue`, `pause`, `terminate`, `disconnect`; events
`initialized`, `stopped`, `terminated`, `exited`, `output`,
`thread`, `breakpoint`.

Capabilities are claimed only when the underlying feature
exists (the draft of record §2 discipline): at M0,
`supportsConfigurationDoneRequest` and
`supportsTerminateRequest`; `supportsSetVariable` when §7.6's
DAP mapping lands (M1, WP-15); conditional breakpoints and
logpoints behind their flags when §5.3/§5.5 land (M2);
`supportsDataBreakpoints` never until M3;
`supportsEvaluateForHovers` only once the strict-mode evaluator
(§7.5, WP-7) lands — flipped in the same change. Acceptance:
golden framed transcripts (the §8 M0 set); at M1 the CLI
fixtures reuse the same scripts, so the two frontends' fixtures
are the same scenarios in two renderings — the §3 discipline,
enforced by the test suite. The zero-JS VS Code extension
(`debugAdapterExecutable` → `rubash --debug --dap`) ships in
M1 (draft of record §10, WP-16).

## 14. Risks and decisions

**Risks**

- *Two audiences, two judgments.* M0 is judged by editors and
  transcripts; the CLI (M1) is where end users will live (the
  owner's expectation, 2026-10-25) and is judged on feel —
  prompt behavior, listing quality, error messages — not on
  seam correctness. Mitigation: WP-14's and WP-9's oracle
  transcripts diff the *renderings* in both frontends, and the
  M1 exit criterion is a human end-to-end session, not only
  green fixtures.
- *Anchor drift on the hottest surface.* The trap/lineno
  machinery is concurrently edited by the #1511 and #1184-f4
  work. Mitigation: P-1/P-2 land first or in parallel on their
  own lanes; WP-1's semantics document is written against
  behavior, with anchors re-verified at WP-3.
- *Condition/watch evaluation surprises.* Full-power explicit
  evaluation (§5.3) means a condition can, in principle, mutate
  the debuggee — as it can in bashdb. The design's defense is the
  explicit/implicit rule, stated in the user docs from WP-11,
  not a sandbox that would break parity.
- *Scope gravity toward gdb.* Every debugger grows gdb-isms
  users don't need in a shell (value history, registers,
  disassembly analogues). §10's No rows are the pre-committed
  refusals; new requests argue against them explicitly.

**Decisions** (options + recommendation + ruling). All five questions
were decided on **2026-10-25**: the owner approved this design as
written, adopting the recommendation recorded under each question. The
recommendation text is kept verbatim, with the ruling appended.

- **Q1 — Entry-stop default.** Does `--debug script.sh` stop at
  the first command (bashdb does), or run to the first breakpoint
  (gdb's `start` vs `run` distinction)? *Options:* (a) stop at
  first command; (b) run until breakpoint, with a `--debug=entry`
  / session option for (a). *Recommendation: (a)* — bashdb
  parity, and a debugger that might run your whole script before
  you can set a breakpoint is a debugger nobody trusts twice.
  **Decided 2026-10-25: (a)** — stop at the first command (bashdb
  parity) is the entry-stop default for both frontends.
- **Q2 — Module placement vs the core split** (§12 P-6).
  *Options:* (a) wait for the split, land in `rubash-core`;
  (b) build in-tree now, move wholesale at the split; (c) build
  the session core (no executor contact) now, the consult after
  the split. *Recommendation: (c)*, degrading to (b) if the
  split slips past 1.5.x — the session core is the bulk of
  M0 and none of it moves semantically.
  **Decided 2026-10-25: (c)** — build the session core now, in-tree,
  shaped for its `rubash-core` home; the funnel consult lands
  with/after the split. Degrades to (b) if the split slips past
  1.5.x. This settles §12 P-6.
- **Q3 — Condition language.** §5.3 evaluates conditions as
  shell commands in-frame (bashdb rule). *Alternative:* restrict
  conditions to the §7.5 whitelist (safer, diverges, and makes
  idiomatic conditions like `condition 1 ((count > 3))` still
  work but `condition 1 grep -q x f` fail). *Recommendation:
  keep the bashdb rule* (full shell command), with the
  implicit/explicit rule carrying the safety story. DAP
  conditions (M2) arrive as strings from a client the user
  controls, same trust domain — no revisit is owed there either.
  **Decided 2026-10-25: the bashdb rule is kept** — conditions are
  full shell commands evaluated in-frame (§5.3); the explicit/implicit
  rule carries the safety story. No whitelist restriction.
- **Q4 — Trace mode's sink.** `trace` (§10) prints `PS4`-style
  lines; to the terminal, or through the debuggee's xtrace fd
  machinery (`BASH_XTRACEFD`)? *Recommendation:* the xtrace fd
  machinery — one tracing stream per shell, composable with the
  user's own `set -x` regions, and the fd discipline (§9.5)
  already distinguishes the streams.
  **Decided 2026-10-25: the xtrace fd machinery** — trace mode writes
  through `BASH_XTRACEFD`, one tracing stream per shell.
- **Q5 — Windows console specifics for the command stream.**
  §9.5 says "the controlling terminal"; on Windows that is the
  console device, and niubash-hosted sessions add a host layer
  (the engine may be embedded, its stdio owned by the host).
  *Options:* (a) define the command stream as an engine-level
  injectable (host may supply it — an embeddable-API surface);
  (b) always open the console device directly. *Recommendation:
  (a)* — it is the same typed-bridge direction as the 1.5.0 RFC,
  and (b) breaks exactly the embedded case 1.5.0 exists to
  serve. (The DAP view has no such question — its stdio *is*
  the transport, owned by the launching host by definition;
  Q5 concerns the interactive view only.) Needs one design
  pass with the niubash host behavior in hand before WP-5
  (the CLI package, M1) closes.
  **Decided 2026-10-25: (a)** — the command stream is an engine-level
  injectable (the host may supply it — an embeddable-API surface in
  the #595 sense); the console device is the default when no host
  supplies one. The pre-WP-5 design pass with the niubash host
  behavior in hand stands as an implementation task, not an open
  question.

## 15. Decision record (provenance)

Every positioning statement in this design traces to one of
these; anything not listed here is engineering judgment, labeled
as such in place.

| Date | Source | Decision |
| --- | --- | --- |
| 2026-10-10 | #497 body | Consensus D1–D6: seam substrate + return-site patch point, stop-and-diff watchpoints, single-thread / synthesized-frame model, evaluate whitelist, gating / state unification, bashdb-oracle acceptance. Original milestone sketch (CLI in M0, `--dap` in M1) — reordered by DP1 below. |
| 2026-10-13 | #497 comment (PR #582 merged) | `docs/design/debugger-m0.md` becomes the M0 draft of record: DAP-over-stdio frontend, one same-process topology, executor PC-hook breakpoint injection (the parse-tree walk serves `verified: false` resolution only). DP1 / DP2 / DP3 posed to the owner. |
| 2026-10-18 | #497 comment (owner ruling) | DP1 = DAP-first (CLI immediately after). DP2 = `--debugger`, GNU spelling, debugger-ready assembly (superseded on the `--debug` point by 2026-10-23). DP3 = line-level + function-entry granularity, two tiers (data-assertion-level stops may be added during implementation). Implementation scheduled for 1.5.0, after the core split. |
| 2026-10-23 | #497 comment (owner ruling, final on flags) | `--debug` is a GNU legacy option and is adopted; `--dap` is a native citizen; **both flags, one option-parse meaning, both naming the project's own native debugger**; `--debugger` stays bashdb/GNU, unmoved. |
| 2026-10-25 | Owner, review chat | The two frontends are one thing — one session, two views; `--debug` is the surface end users will actually use. Recorded as a usage expectation (§1, §14); the build order remains DP1. |
| 2026-10-25 | Owner approval (this finalization) | This design is approved as written, together with the #595 Session API RFC, as one 1.5.0 implementation program. §14 Q1–Q5 decided per the recommendations: Q1 = (a) stop at first command; Q2 = (c) session core in-tree now, consult with/after the split (settles P-6); Q3 = bashdb rule kept (full shell command conditions); Q4 = trace sink via the xtrace fd machinery; Q5 = (a) engine-level injectable command stream. All earlier rulings stand unchanged. |
