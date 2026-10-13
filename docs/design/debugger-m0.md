# `rubash --debug` M0 — Native Script Debugger, Minimal Design

> Design draft for rubash#497 ("Native script debugger (`rubash --debug` /
> `rubash --dap`): M0 design consensus"). This document turns the recorded
> consensus into an M0 that can be started as-is: pinned scope, host form,
> stop seams with file:line anchors, the breakpoint-injection recommendation,
> non-goals, acceptance, and a scale estimate. Three points still need an
> owner ruling before implementation opens — [DP1](#11-decision-points-for-the-owner),
> [DP2](#11-decision-points-for-the-owner), [DP3](#11-decision-points-for-the-owner)
> in the closing section. All anchors verified against `7f18b44b`.

## 0. Scope, pinned

M0 delivers exactly one debuggee topology and one protocol frontend:

- **Topology**: the debuggee is the script, running **in the same OS process
  as the debugger** (pdb/debugpy model — the interpreter is the substrate).
  `rubash --debug script.sh` initializes the engine, starts a DAP server on
  stdio, and executes `script.sh` under stop-event control. No second
  process, no ptrace-style attach, no network transport.
- **Frontend**: DAP (Debug Adapter Protocol) over stdio, Content-Length
  framed JSON — the VS Code / Neovim dap / Emacs dap-mode / JetBrains / Zed
  ecosystem for free. The gdb-grammar CLI from the #497 consensus
  (`b file:line`, `bt`, `n`/`s`/`finish`, `c`) is a *pure view* over the same
  engine event stream and is unchanged in design; its delivery order relative
  to the DAP adapter is DP1.
- **Engine substrate**: stop events at the existing `run_debug_trap` seam
  family plus the function-return patch point (§3). Both frontends are views
  over one engine-owned event stream — the adapter holds zero semantics
  (same architecture rule as rubash#186).

## 1. Host form and process model

```
editor (DAP client)  <--Content-Length JSON over stdio-->  rubash --debug script.sh
                                                            ├─ DAP server loop (M0 new)
                                                            ├─ debugger session state (M0 new)
                                                            └─ engine (existing) executes script.sh
```

- The host flag already parses: `--debug | --debugger` at
  `src/invocation.rs:160-161`. Today it means "enable extdebug"
  (`src/invocation.rs:251-254`, GNU `bash --debugger` shape,
  `src/main.rs` script paths unchanged). M0 repurposes this flag as the
  debugger host entry — what that means for the extdebug alias is **DP2**.
- Startup order: parse CLI → build `Executor` (existing init) → install
  session + breakpoints → DAP `initialize`/`launch` handshake on stdio →
  `configurationDone` → run the script through the existing
  `script_driver` path (`src/main.rs:459` script entry,
  `src/script_driver.rs` line-oriented reader), which needs zero changes —
  stop checks ride the executor seams, not the reader.
- The server loop runs on the executor's thread between stop events; DAP
  requests that arrive while running (`pause`, `disconnect`, `terminate`)
  are the only ones answered immediately (`pause` sets the stop flag, honored
  at the next seam — M0 has no preemption inside a long builtin call; this
  is stated behavior, matching bashdb).

## 2. DAP surface for M0

Requests: `initialize`, `launch`, `attach`, `setBreakpoints`,
`setExceptionBreakpoints` (accepted, zero filters claimed — VS Code sends it
unconditionally), `configurationDone`, `threads`, `stackTrace`, `scopes`,
`variables`, `evaluate`, `pause`, `continue`, `next`, `stepIn`, `stepOut`,
`terminate`, `disconnect`. Events: `initialized`, `stopped`, `terminated`,
`exited`, `output`, `thread`, `breakpoint`.

Capabilities M0 **claims** (nothing beyond this list — never claim an
unimplemented `supports*`):

- `supportsConfigurationDoneRequest`
- `supportsTerminateRequest`
- `supportsSetVariable: false` (M1), `supportsConditionalBreakpoints: false`
  (M2), `supportsDataBreakpoints: false` (write interception is a post-M3
  workstream per the #497 consensus D2), `supportsEvaluateForHovers: false`
  until the strict-mode evaluator lands — flip it in the same PR.

`launch` vs `attach`: both requests map onto the single supported topology —
`launch` spawns the script in-process after `configurationDone`; `attach` is
honored only for the same-process spawn shape (the engine session this host
already owns, e.g. a host started with entry-stop), and rejected with a
clear message for anything else. Background jobs are separate OS processes
(the `&` path re-execs rubash, `src/executor/compound_exec.rs:340-344`,
traps stripped at `:364-378`) and are **not** attachable in M0.

Transport: hand-rolled minimal Content-Length framing + JSON writer/parser
for the request/event subset. The repo has no serde/serde_json dependency
(`Cargo.toml`); the #497 consensus rules out pulling the full serde stack.
No new dependencies in M0.

## 3. Engine substrate: stop seams and events

### 3.1 The seam set is the existing DEBUG-trap distribution

GNU's stop-site set is exactly its DEBUG-trap distribution; rubash already
mirrors it. The debugger stop check sits at the same sites — no new site
enumeration needs inventing (C anchors from #497 consensus D1; rubash
anchors as of `7f18b44b`):

| Stop site | GNU C anchor | rubash seam |
| --- | --- | --- |
| every simple command | `execute_cmd.c:4506` | `src/executor/ast_exec.rs:541` (admission gate `:532-539`) |
| `[[ ... ]]` / `(( ... ))` in compound contexts | `execute_cmd.c:4153` / `:3920` | `src/executor/compound_exec.rs:334` |
| `time`-prefixed compound body | `execute_cmd.c:4506/3920/4153` | `src/executor/compound_exec.rs:982` |
| arith-for init / test / update | `execute_cmd.c:3187` | `src/executor/compound_exec.rs:1135 / :1170 / :1232` |
| `case` head, once | `execute_cmd.c:3668` | `src/executor/compound_exec.rs:2305` |
| function entry, before body | `execute_cmd.c:5383-5391` | `src/executor/function_calls.rs:531` |
| `for` head, per iteration | `execute_cmd.c:3053` | `src/executor/loop_select.rs:117` |
| pipeline element | `execute_cmd.c:4506` (forked child) | `src/executor/pipeline_exec.rs:672` |
| `select` head | `execute_cmd.c:3528` | `src/executor/select_exec.rs:64` |

All eleven funnel into one function: `run_debug_trap` at
`src/executor/trap_exec.rs:716`. `while`/`until`/`if`/`{...}`/`(...)` heads
never stop (GNU has no `run_debug_trap` there); their components stop as
inner commands. Stops fire **before word expansion** (already true at the
seams — e.g. `compound_exec.rs:2292-2305` documents the case-head order).

### 3.2 Ordering contract at each stop

GNU `trap.c:1263 run_debug_trap`: lineno update → `BASH_COMMAND` set → user
DEBUG trap runs in full → extdebug skip/force-return decision
(`execute_cmd.c:4495-4511`, rubash `src/executor/trap_exec.rs:773-775`:
`extdebug && trap_status != 0`) → **debugger stop** → execute. One dispatch
channel only — the debugger check is inserted after the extdebug decision
inside `run_debug_trap`'s caller-visible path, never as a second pre/post
hook stream. The extdebug skip-site parity this builds on landed in the
Wave3 batch (#569); the fire-site contract (`Some(trap_status)` skip return)
is documented at `trap_exec.rs:703-714`.

### 3.3 Gating and state (consensus D5, concretely)

- One dedicated `AtomicBool` (session active) checked **before** the
  existing `has_active_trap_action` consult (`src/executor/ast_exec.rs:532`,
  today via `debug_trap_active`): flag-off cost is one load plus a
  well-predicted branch per seam. The startup/loop perfbudget and the ≤512KB
  release-binary ratchet from #497's release gates must land as CI checks in
  the M0 PR series — `ci.yml` has no budget jobs today, so M0 adds them.
- Unify with the existing debug state — `debug_trap_running`
  (`trap_exec.rs:720,723`), `debug_trap_in_scope` (`trap_exec.rs:842`,
  includes `source_debug_suppressed` and `host_internal_depth`), functrace
  inheritance (`function_calls.rs:476-489`) — so trap and breakpoint
  semantics never diverge.
- The no-recursion invariant extends to debugger stops: no stop inside a
  trap action re-enters the DEBUG path (`trap_exec.rs:720` guard pattern,
  mirroring `SIG_INPROGRESS` at `trap.c:1270`); the stop at a seam inside a
  trap body is suppressed, and the frame transition is reported by the
  return-site event (§3.4) instead.
- Snapshot reads (`variables`, `stackTrace`, `evaluate` while stopped) run
  outside executor flow against the paused `Executor` — re-entrancy is
  structurally impossible because the executor thread is parked in the stop.

### 3.4 The function-return patch point (consensus D1's hole)

With extdebug, a DEBUG-trap status 2 longjmps from inside
`run_debug_trap` (`trap.c:1310-1316`); a beside-hook never executes. M0
therefore also emits the frame-transition event at the RETURN site —
`run_return_trap` at `src/executor/trap_exec.rs:781`, invoked at function
unwind (`trap_exec.rs:1361`) — so stepping over a skip still produces a
correct `stackTrace`. This is one additional event emission, not a new seam
family.

## 4. Breakpoint injection: parse-tree line table vs executor PC hooks

Two candidate mechanisms were considered:

- **A. Parse-tree line table**: after parsing the debuggee, walk the AST and
  build a `(file, line) → node` map; a dispatch loop consults the map before
  each node executes.
- **B. Executor PC hooks**: keep the check at the eleven seams (§3.1), where
  the executor already holds the about-to-run `CommandNode`.

**Recommendation: B — the seam family is the PC; the AST walk is only the
breakpoint-resolution half.** Reasons:

1. **GNU fidelity for free.** The stop-site distribution *is* GNU's
   DEBUG-trap distribution (§3.1). A raw line table would offer stops on
   lines that never fire — a `while` head, continuation lines of a
   multi-line command — exactly the failure #497 D1 flags: those lines must
   answer `verified: false` with "DEBUG trap never fires on this line", not
   stop, and only the seam enumeration knows which lines fire. With B,
   verification at `setBreakpoints` time is decidable from the same
   predicate that decides stopping.
2. **No second semantic channel.** The seams already implement every
   admission rule the debugger needs: the `skips_debug_trap` compound
   deferral list (`ast_exec.rs:494-528`), functrace-gated inheritance
   (`function_calls.rs:476-489`), the no-recursion guard
   (`trap_exec.rs:720`), `debug_trap_in_scope` (`trap_exec.rs:842`). A
   parallel AST-walk checker would have to re-implement all of them and
   would drift — the #117 no-second-admission-list rule, applied to
   debuggers.
3. **The data is already at the hook point.** Each seam holds the
   `CommandNode` with its 1-based source line
   (`src/parser/nodes.rs:1141 CommandNode`, `:1252 pub line: Option<usize>`,
   set at parse time), and file identity is one call away:
   `current_bash_source()` (`src/executor/dynamic_arrays.rs:687`, backed by
   `shell_state.bash_source_stack`). A hit check is
   `session.hit(file, line)` — an `Option<BreakpointSet>` lookup.
4. **What A is still used for**: `setBreakpoints` handling builds the
   file's line set *before* execution by walking the parsed AST once
   (`CommandNode.line` over the tree) and intersecting it with the seam
   line set, so `verified: false` answers immediately for dead lines. The
   walk is metadata-only; it never executes or reorders anything.

Mechanically: `run_debug_trap` gains a session consult after the extdebug
skip decision (§3.2). When the session wants a stop, it parks the executor
thread in the DAP server loop (answering `stackTrace`/`scopes`/`variables`/
`evaluate` against the live executor) and resumes on `continue`/step. Stepping
is implemented as: resume with a one-shot "stop at next eligible seam"
armed flag, filtered by seam kind (`next` skips callees by frame depth
comparison, `stepIn` does not, `stepOut` arms the return site §3.4).

## 5. Frames, scopes, variables, evaluate — reuse of landed structures

`stackTrace`/`scopes` are a rendering of data the engine already maintains
dynamically; no new frame bookkeeping is introduced in M0:

- **Frame synthesis**: FUNCNAME / BASH_SOURCE / BASH_LINENO views —
  `funcname_stack` (`src/executor/dynamic_arrays.rs:683`, backed by
  `shell_state.function_name_stack`), `current_bash_source()`
  (`:687`), `bash_lineno_view()` (`:409`) — zipped into one DAP stack per
  frame. The main-frame rule is already settled: the synthetic `main` frame
  exists only for script-file mode (GNU `shell.c:1635-1650 shell_execscript`;
  `has_script_main_frame` at `dynamic_arrays.rs:389-395`, the fabrication
  fixed under #519). `caller`-level frame indexing follows the same stacks
  (`src/builtins/caller.rs`), so `stackTrace` and `caller N` cannot disagree.
- **Function-entry line data**: `function_definition_locations` and
  `body_open_line` resolution (`src/executor/function_calls.rs:495-511`) give
  breakpoint targets for `b func` (definition line) and correct entry-stop
  lines; the ERR-trap entry-state parity work (#520, errtrace save/clear at
  `function_calls.rs:474-479`) is the pattern the debugger's own
  entry-state save/restore copies so a stop at function entry cannot leak
  shell option state into the debuggee.
- **Scopes**: `locals` = the innermost `local_var_scopes` frame +
  positional params; `globals` = `shell_state.env_vars` minus the
  `__RUBASH_*` internal keys (already filtered from child envs —
  `compound_exec.rs:348-353` — same filter, one more consumer). Arrays
  (indexed/assoc) render as DAP variable children via the existing array
  storage.
- **Evaluate (consensus D4)**: the checker parses the expression with
  rubash's own parser and inspects the AST — never regex. Strict mode
  (hover/watch, once `supportsEvaluateForHovers` flips): read-only forms
  only; rejects `$(...)`/backticks/proc-subs, `${x:=y}` *and* `${x=y}`,
  `${x:?}`/`${x?}`, `+=`/`++`/`--`. REPL context: full commands.
  `setVariable` is not claimed in M0. Watches in M0 are stop-and-diff only,
  surfaced as `output` console lines (`watch: $x 'a'→'b'`) at each stop —
  labeled "changed since last stop", never "write interception" (D2).

## 6. Relationship to existing surfaces

- **extdebug** stays the shopt-level GNU-compatibility surface (skip-site
  contract, `trap_exec.rs:773-775`, sites completed in #569 Wave3). The
  debugger does not add a second admission list; breakpoint admission reuses
  the existing gate predicates verbatim (#117 rule). The only interaction is
  ordering (§3.2) and the shared state union (§3.3). What `--debug` itself
  means once it hosts the debugger is DP2.
- **bashdb compat** is unchanged and parallel: `docs/bashdb-debugging-rubash.md`
  records the external-bashdb workflow, and bashdb is additionally the M0
  regression oracle (§8). Failing bashdb commands still drive root-cause
  engine fixes; none of that moves into the debugger.
- **`bind -v` engine face (#571)**: the gdb-grammar CLI's prompt (DP1
  deliverable) reuses the engine's readline surface and the bind registry —
  the 48-variable GNU 5.3 `bind -v`/`-V` table and inputrc parsing landed in
  #571 (`src/shell/bind_registry.rs`, `src/executor/bind_builtins.rs`,
  `src/builtins/bind.rs`) — so the debugger CLI inherits inputrc /
  editing-mode configuration instead of shipping a second line editor. The
  DAP frontend does not need readline at all (stdio framing, no TTY), which
  keeps the adapter testable headless in CI.

## 7. Non-goals (M0)

- **No remote attach**: stdio transport only; no TCP, no attaching a foreign
  rubash process, no background-job attach (they are separate OS processes,
  `compound_exec.rs:340-344`).
- **No UI**: no VS Code extension in M0 — any DAP client (or the transcript
  test harness) exercises the adapter; the zero-JS extension is M1.
- **No time-travel**: no record/replay, no reverse stepping, no trace ring
  buffer (that is M3, flag-gated, per the consensus).
- **No write-interception watchpoints**: stop-and-diff only, no
  `supportsDataBreakpoints`; the VarTable funnel refactor is a separate
  prerequisite lane (D2), never bundled with debugger PRs.
- **No conditional breakpoints / logpoints / `setVariable`** (M2/M1
  respectively), no exception filters (`err-trap` / `set -e` /
  `parse-error` are M1).

## 8. Acceptance — golden DAP transcripts + bashdb oracle

Every fixture = script + recorded framed DAP transcript + bashdb oracle log,
under `target/issue-suites/results/dap/`. M0 minimum golden set (from
consensus D6, unchanged):

1. `initialize`/handshake + capabilities;
2. `launch` → `configurationDone` → `terminated` lifecycle;
3. `setBreakpoints`: hit + dead-line `verified: false`;
4. `next`/`stepIn`/`stepOut` across a function;
5. `stackTrace` cross-checked against bashdb stop positions and
   `FUNCNAME`/`BASH_LINENO` (per `docs/bashdb-debugging-rubash.md` the
   external bashdb already steps/functions/`where` correctly against
   `target/debug/rubash.exe`);
6. `scopes`/`variables` incl. array children;
7. `evaluate`: safe form + rejected `:=`/`$(...)` forms;
8. `output` categories (stdout/stderr/console).

Plus the three C-level pins from the review: RETURN trap suppressed when
function entry is skipped (`execute_cmd.c:5391`); `line_number` restored on
skipped case/arith (`execute_cmd.c:3670-3675`); `BASH_COMMAND` =
`the_printed_command_except_trap` during trap bodies
(`execute_cmd.c:4502-4505`).

## 9. Scale estimate and release gates

New module `src/debugger/` (four files, no new crates):
`transport.rs` (Content-Length framing + minimal JSON writer/parser for the
subset — no serde), `session.rs` (state machine: running/stopped/terminated,
breakpoint tables, step arming), `dap.rs` (request dispatch, capabilities,
frame/variable rendering), `events.rs` (the engine-facing event enum +
stop parking). Estimated size, anchored to the existing tree
(`src/` is 307 files / 133 in `src/executor/`; the seams and views above
already exist):

- `transport.rs` + `dap.rs`: ~1,200-1,600 LOC (framing, the 17-request
  subset, rendering);
- `session.rs` + `events.rs`: ~600-900 LOC;
- executor touches: one consult after the extdebug decision at the funnel
  (`trap_exec.rs`) plus the return-site emission — each seam itself changes
  by zero-to-two lines because all eleven funnel into one function;
  invocation/script_driver wiring ~100 LOC;
- tests: transcript harness + 8 golden fixtures, ~1,000-1,500 LOC.

Total ≈ 3,000-4,500 LOC across 3-4 PRs: (1) session + seams + events with
the gating flag and perfbudget/size-ratchet CI jobs, (2) transport + dap +
golden set, (3) `launch`/`attach` polish + docs. Release gates from #497
apply to the whole series: flag-off startup/loop within noise (one branch
per seam), release binary growth ≤512KB, M0 touches `src/executor/**` →
captain-exclusive lane.

## 10. M1 outlook (one paragraph)

M1 adds `setVariable`, `exceptionInfo` and the exception filters
(`err-trap`, `set -e`, `parse-error`) — the engine's ERR/EXIT trap plumbing
(`trap_exec.rs:1148` ERR run site, parse-error containment
`trap_exec.rs:513`) already provides the firing points to surface as DAP
exception events; the gdb-grammar CLI (DP1) rides the same session over the
#571 bind surface (§6); the zero-JS VS Code extension ships as
`debugAdapterExecutable` → `rubash --debug --dap` (a flag spelling decided
by DP2); conditional breakpoints and logpoints ride `setBreakpoints` fields
behind their capability flags (M2); background-job attach gets its protocol
design only after the D3 fact (jobs are OS processes) is honored with a real
attach channel; and the VarTable funnel refactor (D2 endgame) lands on its
own lane before any true write-interception watchpoint is claimed.

## 11. Decision points for the owner

- **DP1 — frontend order**: this draft pins the DAP-over-stdio adapter as
  the M0 frontend (owner instruction 2026-10-13), reordering the #497
  consensus milestones (which had the gdb-grammar CLI in M0 and `--dap` in
  M1). The CLI is unchanged in design (pure view, §6) and lands immediately
  after. Confirm DAP-first, or restore CLI-first.
- **DP2 — `--debug` flag semantics**: reuse `--debug`/`--debugger`
  (`src/invocation.rs:160-161`, today an alias that enables extdebug at
  `:251-254`) as the DAP host switch, or introduce a separate `--dap` flag
  and leave `--debug` = extdebug? GNU's `bash --debugger` means extdebug, so
  repurposing is a deliberate compat-surface divergence that needs an
  owner ruling (recommended: `rubash --debug script.sh` = debugger host;
  extdebug stays reachable via `shopt -s extdebug`; `--debugger` keeps the
  GNU meaning).
- **DP3 — M0 stop-site granularity**: full GNU-exact distribution (all
  eleven seams, §3.1 — recommended; the check is one funnel consult so the
  marginal cost of completeness is near zero) or an M0 cut down to simple
  commands + function entry + returns (smaller verification surface, but
  diverges from the bashdb oracle on loop/case/select heads).
