# State-ownership invariants: where "mirrored state, hand-synced" actually costs us

Companion to `fast-path-differential.md` (routes axis, #706) and to the variable-store
axis (#641). This document is the **ownership axis**: which pieces of state have one
owner and which are copies kept in step by hand — and what to do about each.

Version 1 collects only **proven** evidence (file:line from landed reviews, filed
issues and reproduced defects). Items marked *(pending)* are being measured by
in-flight audits and will be appended.

## 0. The test this document applies

A state is *owned* when exactly one component can answer three questions:

1. **who writes it** — one write path, not N call sites;
2. **who reads it** — one read path, or a documented read priority;
3. **what a nested context inherits** — an explicit `clone_for_child` twist, not the
   absence of a reset call.

If any of the three is answered "whoever remembered", the state is *mirrored* and the
defect class is open. The historical tell: **a fix in the seam spawns a sibling issue
in the same seam within days** (recurrence signature), vs. a fix that closes the class
(no recurrence).

## 1. Evidence table

| # | state | write sites | read sites | recurrence evidence | verdict |
|---|---|---|---|---|---|
| A | **traps** (`TRAP` tables as colon-joined strings in `env_vars`, `builtins/trap.rs:9-20`) | reset triggered from **7 sites**: `ast_exec.rs:498`, `command_substitution.rs:902`/`1256`, `compound_exec.rs:1460`, `embedded_mutations.rs:2037`, `pipeline_stages.rs:111`, `read_io.rs:583`; ERR/errexit gates in 4+ | dispatch gate `command_dispatch.rs:262` + per-context readers | #719 fixed #702 by adding a `reset_for_subshell_child` *variant* and de-scoping three paths → **#720 filed four residuals at the same seam within hours** (`f() ( )` flat region, nested-subshell over-fire, procsub, `return`-in-`until` attribution) | **mirrored — worst offender** |
| B | **diagnostic destination** (which stream a message lands on) | `fd_table.write_endpoint(2)` exists (`compound_exec.rs:2318`) but **175 `eprintln!`** outside entry points and direct `io::stdout()/stderr()` in **48 files** | users/consumers; redirection state | #271, #510, #699 R1.2/R1.3 (builtin write-error routing, closed-stderr status) | **mirrored — second** |
| C | **command lookup** (`__RUBASH_HASH_TABLE` serialized, used by `hash`/`type`/`BASH_CMDS`; vs in-memory `CommandLookupCache`, `executor/path.rs`) | both layers are written independently | `find_*` consumers use the cache; user-visible table separate | nibash#392: 5/5 `command not found` for `awk` mid-session, self-healed, coinciding with a `wpm` upgrade that **unlinks and recreates** hardlink targets (`WinuxCmd#1188` root cause) — the two layers can disagree across the swap window | **mirrored — cheap fix available** |
| D | **startup-sourcing handshake** (`__RUBASH_SOURCED_RC`, PR #712) | one write (startup), one read (drivers) | `init_file_needs_direct_run` | introduced by #712 (correct today); if the marker ever escapes to a child shell, the child silently skips its own rc | **mirrored — small, guard it** |
| E | **redirection/fd ordering** | ordered redirect list is static; runtime `exec` rebinds outside it | group/brace prep, function calls | #699/#725 fixed ordering; **#726** records the two leftovers: function-**definition**-level dup splice lost, and runtime `exec` escaping the group's ordered state | **partially owned** |
| F | **nested-context inheritance** (fd table, traps, exit codes, diagnostic ownership per `( )`/`$( )`/pipeline stage/job) | — | — | audit in flight (#713) | *(pending)* |

### Not mirrored — do not "fix"

The forkless transport substrate is a legitimate design, not an anti-pattern:

- the variable **dual-store** (typed cell + mirror string) — the reader-side rule is
  already proven (#638); #711 is the counter-example: "unifying" the fd-0 cursor to raw
  units would have *introduced* a bug;
- `__RUBASH_FD_*` (36 sites / 10 files) and `FUNCTION_STDIN(_OFFSET)` (165 refs /
  24 files) — GNU gets inheritance from `fork()`; rubash has no such primitive. These
  can be **typed**, not eliminated.

### Precedent that the frame works

`ShellState` landed through exactly the quantification approach used here: comparing
the hand-written lists (7 vs 55 drift) exposed the alias/function leak. **Zero
recurrences** in that class since.

## 2. Ownership plan (incremental boundary, in order)

1. **P1 — trap owner** (`ShellState` + one typed `clone_for_child_save` twist).
   Fold the `command_dispatch.rs:262` attribution gate in as one semantic. This is the
   highest-yield step because A is where the recurrence is *measured*, not inferred.
2. **P2 — diagnostic channel**: every write goes through the fd table's endpoint for
   the command's *ordered* state. 175 `eprintln!` + 48 files to converge; a
   grep-guard test can pin the direction from day one.
3. **P3 — lookup invalidation contract**: on `ENOENT`/`FILE_NOT_FOUND` for a cached
   absolute path, bypass the cache once, re-resolve, and **do not cache the miss**.
   Independent of P1/P2, small, immediately testable with a swap-exchange trigger
   (the deterministic repro being built for niubash#392).
4. **P4 — startup facts**: turn `__RUBASH_SOURCED_RC` into a typed record with an
   explicit "never inherited" rule (a one-line assertion moment, not a redesign).

Explicitly **not** in this plan: dual-store redesign, transport-mirror removal.

## 3. Guardrails (why this is not whack-a-mole)

- **One new invariant test per bucket**, in the same PR that unifies it:
  - traps: every reset site must obtain state from `TrapOwner::for_child()`; a source
    guard test fails if a new `reset_*` call site appears outside it.
  - diagnostics: a source guard fails if a new `eprintln!`/direct stream write appears
    outside the entry-point list.
  - lookup: the swap-exchange trigger runs in CI (deterministic, bounded).
- **Recurrence is a measurable assertion, not an opinion**: if a fix in one of these
  seams is followed by a sibling issue in the same seam, the fix is re-opened as
  "class not closed" — exactly what happened with #719 → #720, recorded as the model.
- **Serialization with the perf line**: P1 touches `state.rs`/`var_table.rs`, shared
  with #616/#618 compensation/budget work — the repo has a documented M5/S13 merge-loss
  history; P1 lands between perf phases or is explicitly serialized.
- **Each step lands alone.** No mixed PR; the invariant test is the acceptance.

## 4. Decisions requested

1. Accept **P1 (trap owner)** as the first boundary to close? (If yes, the #720
   residuals should be re-scoped under it instead of patched individually.)
2. Accept **P2** as the diagnostic-ownership direction (the cheap part is the
   grep-guard; the migration can be incremental)?
3. **P3** is independent — may be dispatched immediately (it already has a filed
   cross-repo trigger from niubash#392).
4. Is the *recurrence signature* acceptable as a formal re-open criterion for the
   trackers' "closed" states?

## Appendix — provenance

- #717 (this brief's home), #719/#702/#720 (trap seam), #271/#510/#699 (diagnostics),
  niubash#392 + WinuxCmd#1188 (lookup vs wpm swap), #712 (startup handshake), #699/#725/#726
  (redirection ordering), #638 (dual-store reader rule), #641 (variable-store axis),
  #706 (routes axis), #713 (nested-context axis, pending), #711 (cursor counter-example).
