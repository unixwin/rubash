# #2245 — comsub ShellState COW: design evaluation (2026-10-31)

**Verdict: DESIGN-ONLY.** The design is evaluated to implementation-ready
precision (Tier-2 blueprint in §5), but the change does not close as a
small patch: the issue's own charter assigns it to the post-1.5.0 Session
state line as a standalone project with a full mutation-point audit and
acceptance pins, and the dual-store unification evaluation must sequence
first (§6). This document is that project's scoping record; no product
code was changed by the evaluation.

Evaluation base: d758fd0. Current master (711ad72e) delta in
the four relevant files is small and additive (state.rs +30 lines,
command_substitution.rs +82, var_table.rs / variables.rs unchanged), so the
architectural conclusions carry to master unchanged.

## §1 Mechanism — code-proven

- Every general-path command substitution builds its child executor by
  deep-cloning the entire shell state:
  `command_substitution_executor` (src/executor/command_substitution.rs:1740)
  does `let mut shell_state = self.shell_state.clone();` plus
  `self.fd_table.clone()`. Only the word-shortcut fast path avoids it.
- `ShellState` (src/shell/state.rs:72) is ~55 fields: the typed variable
  store, the mirror VarTable, cwd, four function maps, alias map, positional
  params, PIPESTATUS vector+shape, six call-stack vectors, three local-scope
  stacks, job table, completion registry, history, and ~15 `Cell`/`RefCell`
  interior-mutable latches.
- The clone is hand-written (`impl Clone for ShellState`, state.rs:647) and
  encodes deliberate per-field exceptions:
  - `bind_registry`: the SHARED handle, not a copy (process-global in GNU).
  - `session_history`: deep-copied into a fresh `Rc<RefCell<..>>`
    (isolation is load-bearing).
  - `procsub_streams`: map cloned, stream values shared by `Rc` (fork
    shares the open file description).
  - `procsub_adhoc_carriers`: clone starts EMPTY (#1691).
  - every `Cell`/`RefCell` scalar: re-wrapped fresh with the copied value.
  - `random_state`: cloned via `clone_state()`, reseeded by the caller.
- Clone boundaries in the whole engine (enumerable, 6): the comsub clone
  above + five `clone_for_child_save` save/restore users (ast_exec.rs:967
  `( )` regions, compound_exec.rs:3493, embedded_mutations.rs:2186/2431,
  external_finish.rs:390). Any COW design only has to be right at these 6
  sites plus the stores' own funnels — the boundary count is NOT the
  problem; the mutation surface behind the boundaries is (§4/§5).

## §2 Cost evidence

- On record (issue, from the perf-2229-split measurements, quiet machine,
  release): variable-family slope ≈ 0.57 µs/variable; at 600 variables the
  clone is ~340–530 µs per comsub round; the #2229 combo shape
  (`producer | while read` + per-round `$(typeset -p $var)`) is ~759 µs/round
  with the clone as the dominant term.
- Corroboration (this pass, NOT bankable as a slope): the evaluation sweep
  (variable-count generator + ABBA-interleaved median runner, kept with the
  evaluation's working notes) was re-run
  against the current-master release binary under load ≈ 9 (the box was
  running the parallel out-of-gate restart). Per-comsub cost at N=0..1000
  measured 1.5–5.8 ms — the cost is unambiguously present and large, but
  load spikes swamp the per-variable slope (least-squares 3.7 µs/var,
  N=400 dipping below N=0). The issue's quiet-machine 0.57 µs/var stands as
  the authoritative number; a quiet-window re-baseline is step 0 of the
  project (§7).

## §3 The GNU model being emulated

GNU gets this for free: `command_substitute` (subst.c:7143) forks, and the
kernel's copy-on-write page sharing means the child pays only for pages it
actually writes; a read-only substitution body costs ~nothing in state
copying. rubash's flat in-process model has no fork, so the deep clone is
its emulation of that page copy — paid in full, up front, on every
general-path comsub regardless of whether the body writes anything. The
design goal is therefore precisely: reproduce the kernel's pay-on-write
profile at store granularity. (Behavior facts only; no GPL code used.)

## §4 Tier 1 — whole-`ShellState` `Rc` + `make_mut`: REJECTED

Three independent blockers, each sufficient:

1. **Interior mutability defeats `make_mut`.** ~15 fields are `Cell`/
   `RefCell` mutated through `&ShellState` (word expansion holds
   `&Executor`). Under an `Rc<ShellState>`, those writes never pass a
   `&mut` choke, so `Rc::make_mut` never fires and the child would write
   the parent's latches directly. The codebase already documents this:
   `InteriorSnapshot` (state.rs) exists because "an in-place command
   substitution can only mutate state behind `Cell`/`RefCell`".
2. **No mutation choke for plain fields.** `shell_state` is a pub(crate)
   field touched at 2,888 sites in 94 files, with 132 direct plain-field
   writes (`shell_state.<field> = ...`) and pervasive direct map access.
   There is no `shell_state_mut()` accessor to convert to `make_mut`;
   creating one is a rewrite of the executor's state-access idiom, not a
   contained change.
3. **Deliberate sharing exceptions.** A blanket share breaks the Clone
   impl's per-field decisions (§1): `bind_registry` must stay shared,
   `session_history` must stay isolated-but-host-owned (below),
   `procsub_streams` values must stay shared, carriers must stay empty.

## §5 Tier 2 — per-store COW: funnel audit PASSES, blueprint

Wrap only the heavy stores; the `ShellState` struct itself is still cloned
field-by-field exactly as today (Cells re-wrapped fresh, exceptions kept).

Funnel audit (the load-bearing claim — mutation cannot bypass):

- **`VariableStore`** (typed store, BTreeMap<String, Variable>): all
  mutation goes through inherent `&mut self` methods; `src/shell/
  variables.rs` contains NO `RefCell`/`Cell` at all (no interior
  mutability); `Variable` is a plain value struct (no `Rc` inside), so
  map-level COW has clean value semantics. `get_mut` hands out
  `&mut Variable` — safe iff it first unshares, which `&mut self`
  guarantees it can.
- **`VarTable`** (mirror store): writes are funneled BY DESIGN — `Deref`
  to the values map is read-only and there is deliberately **no
  `DerefMut`** ("P1a dirty choke-point: every mirror write observes
  here", var_table.rs:181+). Its six sub-fields (values, attrs, tracked,
  stale, mirror_epoch, synced_epoch) form one consistency unit, so COW
  must wrap the WHOLE VarTable, never the values map alone.
- **functions / aliases maps**: plain `ShellState` fields; direct write
  sites are enumerable — exactly 9 (`functions.insert` function_calls.rs:140;
  `aliases.remove`/`functions.remove` state_views.rs:665/678;
  `functions.remove`/`aliases.remove` unset_arrays.rs:147/650;
  `aliases.clear`×3 + `aliases.insert` script_driver.rs:6349/6453/6481/6565)
  plus Executor helper funnels. Convertible in the same pattern.
- **Excluded from v1**: `session_history` (in interactive sessions the
  niubash HOST owns the list and the engine's `Rc` is a view onto it —
  COW here can diverge host and engine; needs its own design),
  `job_table` / `completion_specs` / stacks / scope stacks (small at
  realistic sizes; they form the residual N=0 intercept).

Blueprint:

1. `VariableStore { variables: Rc<BTreeMap<..>>, .. }` — every `&mut self`
   method calls `Rc::make_mut` first; `Clone` becomes a bump.
2. `VarTable { inner: Rc<VarTableInner> }` — same at its choke methods;
   `Deref` reads through the `Rc`.
3. functions/aliases (and optionally the three function-metadata maps):
   field type becomes a small `CowMap` wrapper or `Rc<HashMap<..>>` with
   `make_mut` at the 9 sites + helpers.
4. `impl Clone for ShellState` keeps its exact current shape; the wrapped
   fields' clones become bumps. `clone_for_child_save` and the comsub
   constructor need NO changes.
5. Semantics, honestly stated: a read-only child pays ~zero (the #2229
   combo shape — the actual complaint — is read-only: full win). A
   writing child pays ONE deep clone at its first write — the same cost
   as today, deferred; never more. Save/restore boundaries get the same
   profile.

## §6 Why this is not implemented in this finisher pass

1. **Charter.** The issue itself scopes this as a standalone post-1.5.0
   Session-state-line project "连同突变点全审计与验收钉实施" (with the
   full mutation-point audit and acceptance pins). §5 is the scoping pass
   of that audit — it proves the two stores are funneled, but the
   exhaustive pass the charter demands also covers the tempenv
   save/restore carriers, the #2058 respawn-transport census paths,
   the Session §5.3 state views, and the debug-only cell verifier
   (VariableStore::cell_candidates), none of which a grep-level funnel
   proof closes.
2. **Sequencing vs single-store unification.** The `variables` +
   `env_vars` dual store is explicitly transitional ("kept in sync …
   during the ongoing typed-store migration"), and the single-store
   unification is slated for evaluation on the same Session state line.
   COW-ing VarTable's six-field freshness apparatus now risks building
   on a structure that unification deletes. The typed-store half (§5.1)
   survives unification either way — if any step is taken early, take
   ONLY that half.
3. **Unverified projection.** The 759 → ~300 µs/round expectation assumes
   the variable slope dominates the #2229 shape; the residual intercept
   (functions/history/stacks) has never been attributed on a quiet
   machine. Committing scope before that measurement repeats the
   debug-vs-release phantom class of error this project has been burned
   by before.

None of these are "cannot" — they are "not as a same-day patch to the
engine's central state type, verified only by cargo check + CI, on a
loaded box". That is exactly the contained/uncontained line the issue
draws.

## §7 Acceptance net — ready and verified

- **Isolation battery** (22 faces: scalar/array/assoc/unset/fresh/
  functions/aliases/options/traps/cd/positional/readonly/export-attr/
  locals/fd/umask/nameref/integer-attr/nested/arith/jobs; script kept
  with the evaluation's working notes — step 1 of the project should
  land it as an in-repo regression target): re-run this pass — output
  **byte-identical** between GNU bash 5.3.20 (oracle) and current
  master 711ad72e release (24 lines). It is the project's entry gate:
  any COW step must keep it byte-identical, plus GNU-vs-post-change on
  every face.
- **Sweep**: the evaluation's variable-count sweep (ABBA medians) —
  step 0 is a quiet-window before/after baseline at
  N ∈ {0,100,200,400,600,1000}.
- Full regression + lib suites and the fd-scrubbed pin batches per the
  collector protocol, as with every state-boundary change.

## §8 Recommended project shape (for the Session state line)

0. Quiet-window baseline: clone-cost attribution (variables vs mirror vs
   functions vs history vs rest) — decides whether §5.3 is in scope.
1. VariableStore COW (§5.1) + isolation battery + pins named issue2245.
2. VarTable COW (§5.2) — only if unification (§6.2) has ruled by then;
   otherwise fold into the unified store's COW.
3. functions/aliases CowMap (§5.3) if step 0 says the intercept matters.
4. session_history sharing design (host-ownership aware) — separate.
5. Close-out measurement vs the §2 baseline; update the issue with the
   per-shape table (read-only vs writing comsub separated).
