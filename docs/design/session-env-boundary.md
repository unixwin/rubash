# Session ↔ process-environment boundary: inventory, adjudication, and the InvocationShape construction face

Status: successor section to the 1.5.0 Session API design
(`docs/design/1.5.0-session-api.md`, FINAL 2026-10-25). That document
is frozen; nothing here edits it. This section closes the two
post-freeze contract items that share one boundary — rubash#2084
(a Session rewrites its host process's environment and signal
dispositions; descendants inherit) and rubash#2130 (the startup
shape lives in user-visible environment carriers) — plus the
inventory half of the family #2001 belongs to. #2078's process-level
parse-cache adjudication is already landed and is not re-opened.

GNU ground rule used throughout: the state this document moves is
state GNU keeps in process-local C globals, never in the
environment — `interactive_shell` / `login_shell` (shell.c:497-503,
540-549), the terminating-signal dispositions (sig.c:226-229,
315-316, 333), the current line for diagnostics (variables.c /
error.c). A rubash process that IS the shell (the CLI form) may
hold that state wherever it likes; an embedded `api::Session` must
not leave it behind in a host that is not a shell.

---

## 1. #2084 — inventory of process-global write points

Scope: every write a `Session`'s lifetime can make to state that
outlives the session's own stores — the process environment block,
kernel signal dispositions, the process umask, the process cwd —
plus the process-global statics the engine maintains. "Session
lifetime" runs from `SessionBuilder::build()` to the drop of the
session's `Executor` (the API's close boundary is `Session::close`,
which drops the executor).

Adjudication rule (owner-directed, conservative): a write a spawned
child can observe as contamination is presumed
**must-scope-or-restore** unless the CLI form provably needs it
process-global. CLI necessity never excuses the embedded form: the
CLI executor is the process-exit form (`Executor::new_process_exit`)
and already skips the drop-time restores, so scoping the library
form costs the CLI nothing.

| # | Write point | Mechanism / site | CLI form | Embedded disposition |
|---|-------------|------------------|----------|----------------------|
| E1 | Exported/user variable mirror writes (`PATH`, `PWD`, `OLDPWD`, `HOME`, `TMPDIR` family, `LC_*`/`LANG`, every `__RUBASH_*` not in E2) | `Executor::set_env`/`remove_env` and the assignment gate mirror into the process env (`set_process_env`, executor/local_helpers.rs); G-03 journal records each lineage's writes | Necessary: children spawned without a composed env inherit the process block; the CLI process is the session | **Restore at drop (already implemented, G-03)**: journal-scoped conditional restore against the birth snapshot. Mid-session visibility to host-spawned children is the mirror's purpose and is documented contract, not contamination: a child the HOST spawns while a session runs is a child of a process whose shell state is live. |
| E2 | Invocation-classification keys (`__RUBASH_INTERACTIVE`, `__RUBASH_INTERACTIVE_FLAG_OFF`, `__RUBASH_IS_C`, `__RUBASH_LOGIN_SHELL`, `__RUBASH_DEBUGGER_MODE`, `__RUBASH_DEBUGGER_ATTEMPTED`) | rubash#2259: never mirrored, never projected into child env, stripped at Executor construction import (`INVOCATION_STATE_ENV_KEYS`) | n/a (never leaves the table) | **Scoped (already implemented, #2259)** at the process boundary. The in-table forgery surface is #2130 — see §2. |
| E3 | `__RUBASH_CURRENT_LINE` process-env entry | Written through `set_env` (source/execution.rs) so builtin diagnostic readers without executor access (`std::env::var` fallbacks in builtins) see the executing line; also composed into spawn envs explicitly | Necessary today for the diagnostic faces that read the process copy | **Journaled mirror (E1 family)**: written via the journaled funnel during runs, conditionally restored at drop. The process-env reader fallbacks are contract surface; moving 45 files of readers to executor state is not this item. Documented, no code change. |
| E4 | Locale category sync (`LC_ALL`, `LC_CTYPE`, `LC_COLLATE`, `LC_MESSAGES`, `LC_NUMERIC`, `LC_TIME`, `LANG`) | `locale::sync_process_locale` writes the table's categories into the process env under the process-state lock | Necessary: the C-library locale is process-wide; GNU's sv_locale watchers fire on assignment (variables.c:4283-4350) | **Documented contract (already adjudicated G-03)**: deliberately NOT journaled — locale is process-wide by C-library nature; per-session locale isolation is workspace-split (WP-3) territory. Recorded here so the exception is a ruling, not an oversight. |
| E5 | Terminal-size sync (`LINES`/`COLUMNS`) | script_driver winsize sync writes the process env **only when the name already exists there** | Necessary for the same conditional reason: it refreshes the host's own exported values | **Documented contract**: a conditional mirror — it never creates the names in a host that did not export them, so it cannot contaminate a host that did not opt in. No code change. |
| E6 | Respawn transport keys (`BACKGROUND_WRAPPER_ENV_KEY`, `ASYNC_CHILD_ENV_KEY` in builtins/kill.rs) | Set in the process env around wrapper spawns; the consumer removes them at claim time (`remove_var` on read) | Necessary: the transport IS the process env of the spawn | **Scoped by construction**: set/consume/remove are paired inside one spawn operation; nothing outlives it. No code change. (Their classification value is separately gated by provenance validation, #2134/#2136.) |
| E7 | `__RUBASH_CMD_FILE` (invocation.rs) | Host invocation machinery sets it before running a `-c`-file form and removes it after | CLI-host face | **Scoped by construction** (paired set/remove in the host path); not reachable from a library Session run. No code change. |
| S1 | SIGQUIT policy ignore (sig.c:333) | `kernel_signals::capture_entry_and_policy` sets SIGQUIT to SIG_IGN and records state, once per process | Necessary: every shell ignores SIGQUIT | **RESTORE AT SESSION END (fixed by this change)**: see §1.1. The S4 battery evidence is this row: a library session applied the policy to its host process and never gave it back, so a later host-spawned child inherited `trap -- '' SIGQUIT`. |
| S2 | SIGPIPE reset to SIG_DFL (rubash#1333) | Same capture step: the Rust runtime's SIG_IGN is reset so the shell dies by SIGPIPE like GNU's | Necessary for the shell process | **RESTORE AT SESSION END (fixed by this change)**: the pre-session disposition (in practice the runtime's SIG_IGN) is part of the §1.1 snapshot. In-process pipeline members keep their per-thread `SigpipeBlockGuard`; the process disposition is restored only when the last session leaves. |
| S3 | Fatal-quartet handlers (SIGSEGV/SIGBUS/SIGILL/SIGFPE) | `install_fatal_handlers` at the same capture step; genuine faults chain to the disposition saved at install time | Necessary: SENT deliveries to the set need shell semantics (rubash#1475/#1511) | **RESTORE AT SESSION END (fixed by this change)**: the saved-at-install chain target is the pre-session handler; restoring the §1.1 snapshot returns it verbatim. While any session lives, the handler's untrapped behavior is observably default (restore-DFL-and-re-raise for sent, chain for faults), so the install is near-transparent to the host. |
| S4 | Trap-table-driven dispositions (managed set) | Boundary reconcile (`reconcile_kernel_trap_dispositions` → `kernel_signals::reconcile`) installs Handler / SIG_IGN / Default per the session's trap table at every command boundary; interactive sessions additionally pin SIGINT/SIGHUP to the mailbox handler and SIGTERM to SIG_IGN (sig.c:315-316 family) | Necessary: this is how a shell process receives its traps | **RESTORE AT SESSION END (fixed by this change)**: whatever the last reconcile installed is undone from the §1.1 snapshot when the last root executor drops. While sessions live, dispositions are process-global by design (one kernel table per process); Q5's single-thread contract plus the process-state lock serialize reconciles. A still-live sibling's traps are never restored out from under it — the restore fires only at the zero crossing. |
| S5 | Signal mailbox registration | Unix: none (no artifact). Windows: a per-pid marker under the temp dir, unregistered at drop | Necessary for cross-process delivery | **Scoped (already implemented)**: drop unregisters (skipped for the process-exit form, whose stale markers self-heal by dead-pid checks). |
| S6 | Entry-disposition capture (`ENTRY_IGNORED`, entry snapshot) | Read-only capture at first capture call | n/a | **Process-birth contract state**: a fact about the process's birth, shared by every session in it; never written back. No action. |
| U1 | Process umask | The `umask` builtin calls `libc::umask` (builtins/umask.rs); the value is also carried as session state (`__RUBASH_UMASK`) and spawn composition reads the session value | Necessary: the CLI process's children inherit the process umask | **RESTORE AT SESSION END (fixed by this change)**: birth umask is snapshotted per root executor (the `snapshot_process_umask` probe exists); the executor journals its own last-set value at the builtin write point, and drop restores the birth value only when the current mask still equals that journaled value (the G-03 conditional shape, so a host or sibling write after the session's last set stands). |
| C1 | Process cwd | The cd machinery moves a **session cwd field** (G-02), never the process cwd; the only `set_current_dir` in the tree is a test restoring after itself | n/a | **Already session-scoped (G-02)**. Spawns receive the session cwd explicitly. No action. |
| G1 | Process-global statics: parse cache (#2078), `RESPAWNED_CHILD`, `STARTUP_FILES_DONE`, host `OWN_*` shape statics | Various; #2078 adjudicated the parse cache's keying; #2133/#2129 moved the host's run-once and shape facts to statics | Necessary in the CLI process (they ARE the process's facts) | **Documented contract**: these are facts about the single process, not about a session; an embedded host gets one process's worth. The API-side respawn fact becomes a construction parameter instead (§2) so a Session does not have to consult the process's static. |
| G2 | Executor birth import | A new Executor imports the process env as its initial table | Necessary: a shell starts from its environment | **Contract**: birth import is the designed seam (the builder's `environment` seeds layer over it). Ambient copies of the E2 keys are stripped at import (#2259/#2137). No action. |

### 1.1 The disposition snapshot/restore mechanism (S1–S4)

`kernel_signals` gains an ownership count for **root executors**
(`Executor::new` lineage roots — not fork copies, not the
process-exit form):

- The first root birth in the process snapshots the kernel
  disposition (`sigaction` struct) of every managed signal and the
  fatal quartet, **before** the entry-policy capture runs, and
  marks the policy applied.
- Root births while the count is non-zero only increment.
- The drop that returns the count to zero restores the snapshot
  verbatim (each saved `sigaction` reinstalled), resets the
  recorded disposition state to match the restored dispositions,
  and marks the policy unapplied — a later root birth re-runs the
  entry-policy capture (the `Once` keeps the entry FACTS:
  `ENTRY_IGNORED` and the fatal handlers' chain target are
  re-derived from the restored dispositions, which are the
  originals, so re-capture is consistent).
- The process-exit executor never counts: the CLI process keeps
  today's behavior bit for bit, including never restoring.

Interleaving discipline is the env journal's (G-03) at coarser
grain: restores happen only when no session remains, so a restore
can never strand a live sibling; sessions that overlap share the
process's one kernel table exactly as they do today.

## 2. #2130 — InvocationShape and the marker read-point census

### 2.1 The construction face

```rust
pub struct InvocationShape {
    pub interactive: bool,
    pub login: bool,
    pub respawn: RespawnSource,
}
pub enum RespawnSource { Fresh, EngineChild }
```

`SessionBuilder::invocation_shape(shape)` declares all three at
build time. The existing `login_shell(bool)` / `interactive(bool)`
builder methods remain and write the same fields. At build, the
declared shape is applied to the executor AFTER the environment
seeds, so the structured parameter — not a stray seed — is
authoritative for the fields it names. The executor gains typed
invocation state (`Executor::invocation_interactive()` /
`Executor::invocation_login()`); `Session` exposes the built shape
back (`Session::invocation_shape()`).

Until every read point migrates, the `__RUBASH_INTERACTIVE` /
`__RUBASH_LOGIN_SHELL` table entries remain as a **write-through
projection**: every funnel that writes the entry (`Executor::set_env`,
`remove_env`, `mark_login_shell`, whole-table restores) also updates
the typed state, and applying the typed state writes the entry, so
unmigrated readers observe the same value migrated ones do. What
changes is authority: engine decisions read the typed state, so an
in-session script assignment to the carrier — which writes the
variable store through the general assignment machinery, not the
funnels — can no longer flip the session's shape. That writability
was the forgery surface #2130 cites (and the inheritance half of it
was #2129, already closed at the host layer; the cross-process
half was #2259).

Respawn: hosts of the CLI form already record respawn as a
process fact (`RESPAWNED_CHILD`, folded at Executor birth from the
spawn stamps). For the API form the shape parameter carries it:
a session built with `RespawnSource::EngineChild` refuses the
startup chain the same way the respawned CLI child does
(`Session::source_startup_files` consults the declared shape, not
only the process static).

### 2.2 Census — `__RUBASH_INTERACTIVE` read points

Classification: **M** = migrated to typed state in this change;
**P** = stays on the table as projection (observable face — the
value rendered must be the carrier's, by definition of the face);
**H** = host face (main.rs / CLI drivers: the host that seeded the
shape reads its own seed back; the CLI form's behavior is frozen by
the hard gates, and the host's startup chain already adjudicates by
its OWN_* statics per #2129); **T** = test seed (setup, migrated to
the funnel where touched — expectations unchanged).

| Site | Class | Note |
|------|-------|------|
| executor/printf_path_builtins.rs (exit echo, bash_logout, logout gate) | M | Executor self-reads; the farewell echo and logout faces are shape decisions |
| script_driver.rs (stdin drivers, finish_shell faces, hangup faces) | M | Driver reads on the session's executor |
| executor/public_accessors.rs (11 presence reads) | M* | Each is an `is_interactive`-family predicate over the session's own table; migrated where the read is a decision. *Presence-vs-value: these sites used `contains_key`; the typed state is set exactly when a funnel writes the key with value `1`, and every production seed writes `1` — the `=0`-present case has no production writer (hosts write `1` or remove). Recorded here as the one semantic tightening: a hand-forged `__RUBASH_INTERACTIVE=0` entry no longer reads as "interactive" at decision points. It still renders in projections. |
| executor/command_execute.rs, command_dispatch.rs, command_prepare.rs, temporary_assignments.rs | M | Dispatch-time decisions over the executing executor's table snapshot — the snapshot is taken from the same executor the typed state lives on |
| executor/trap_exec.rs (5), executor/job_builtins.rs (2), executor/compound_exec.rs, executor/expand_word.rs (2), executor/prompt_expansion.rs, executor/bind_builtins.rs, executor/external_inner.rs, executor/pwd_loop_builtins.rs (2) | M | Executor self-reads (decisions: trap behavior, job notices, expansion gates, prompt faces) |
| executor/parameter_errors.rs (2) | M | Diagnostic-shape decisions inside executor methods |
| builtins/kill.rs `reconcile_kernel_trap_dispositions` | P (residual, explained) | As implemented, this stays on the carrier map: the function sits at the builtin ABI boundary — builtins receive the env map, not the executor, and its interactive verdict feeds the trap-listing reconciliation whose input IS the builtin-visible table. The decision points *around* it (trap_exec's five reads, script_driver's hangup faces) migrated to the typed state; this one map-level read remains because its contract is the map itself. Revisit if the builtin ABI grows an executor handle. |
| builtins/set.rs `builtin_error_prefix`, builtins/set/options.rs (posix walk) | P (residual, explained) | Same ABI-boundary reason: free functions over the env map handed to builtins. `builtin_error_prefix`'s presence semantics are preserved exactly (any carrier entry counts). The posix walk in options.rs iterates the *startup* map to classify the invocation the process was born with — a birth-fact read, not a session-state read, so the carrier is the correct source there. |
| builtins/set.rs `$-` / option rendering | P | `$-` and `set -o` render the option state; interactivity there is the shell-option face, which the option machinery already keeps — the carrier read is the projection of record for that face |
| src/main.rs (all reads) | H | Host reads its own seeds; startup chain adjudicates via OWN_* statics (#2129) |
| executor/tests.rs (7 seeds) | T | `set_env` write-through keeps them working verbatim |
| niubash (consumer) | — | Seeds via the builder and `set_env`; the write-through funnel keeps every seed authoritative. No niubash change. |

### 2.3 Census — `__RUBASH_LOGIN_SHELL`, `__RUBASH_INTERACTIVE_FLAG_OFF`, `__RUBASH_STARTUP_FILES_DONE`

| Marker | Sites | Verdict |
|--------|-------|---------|
| `__RUBASH_LOGIN_SHELL` | printf_path_builtins.rs ×3 (M, with §2.2); script_driver.rs ×4 startup/logout faces (M); public_accessors `mark_login_shell` (the funnel — now also sets typed state); main.rs reads (H); init.rs birth strip (stays: it is the import boundary) | Typed state + projection, same rule as INTERACTIVE |
| `__RUBASH_INTERACTIVE_FLAG_OFF` | api/session.rs run wrapper, prompt_command.rs, main.rs REPL wrappers (write); printf_path_builtins.rs (read, beside INTERACTIVE) | **Stays a carrier, named reason**: it is not construction shape but a run-scoped suppression toggle, written and cleared inside single runs (prompt-command execution, API `run()`), with no cross-run or cross-process meaning. Its read point migrates only in its INTERACTIVE half. Forgery impact of the toggle itself: a script that sets it suppresses its own session's farewell-echo face for that run — self-inflicted cosmetics, no classification power. |
| `__RUBASH_STARTUP_FILES_DONE` | main.rs only | **Already retired** (rubash#2133): the carrier is a process-local static now; the remaining mentions are the host's startup strip list. Census zero. |

### 2.4 Marker fate

- `__RUBASH_INTERACTIVE` / `__RUBASH_LOGIN_SHELL`: demoted to
  write-through projections. They remain visible to the faces that
  render them (option state, `$-`-adjacent faces, child-table
  composition inside the session) and disappear from the decision
  path as §2.2's M rows land. They were already invisible outside
  the process (#2259).
  Forgery surface, verified from the assignment machinery: a script
  assignment (`__RUBASH_INTERACTIVE=1`, including the temporary
  form) writes `env_vars` directly — the assignment executor and
  its restore path never pass through `Executor::set_env` — so a
  forged entry lands in the *projection* (a child composed from
  this table would inherit it; the §2.2 P-residual readers still
  see it) but never in the typed state, and every migrated decision
  point is immune. Pin: `issue2130_forged_carrier_assignment_does_not_flip_decisions`.
- `__RUBASH_INTERACTIVE_FLAG_OFF`: stays a carrier (§2.3).
- `__RUBASH_STARTUP_FILES_DONE`: retired (already).

## 3. Flagged for the collector's ruling (not adjudicated here)

1. **E4 locale** is recorded as documented contract per the G-03
   ruling on file; if the owner wants per-session locale isolation,
   that is a new ruling, not a fix.
2. Any §2.2 point found during migration whose map is **not** the
   executing executor's own table (a genuinely foreign snapshot)
   stays on the carrier with its reason added to the census —
   the census must reach zero *unexplained* residuals, and a
   foreign-snapshot read is an explained one.
3. New public symbols for the API ledger pairing (the ledger file
   itself is NOT edited here — it must stay byte-identical across
   the two repos): `rubash::api::InvocationShape`,
   `rubash::api::RespawnSource`,
   `method rubash::api::SessionBuilder::invocation_shape`,
   `method rubash::api::Session::invocation_shape`,
   `method rubash::executor::Executor::invocation_interactive`,
   `method rubash::executor::Executor::invocation_login`.
