# Indexed array structured store: measured Phase 1a for #616

## Scope and GNU ownership

Baseline: `6daf8e1`. Candidate branch: `perf/array-structured-store`.

`#616` is the umbrella issue for the array subsystem's O(n) string round trip.
Phase 1a has exactly one deliverable: make `${#a[@]}` O(1) by giving the typed
store a structured indexed cell, and do it with **zero behavioural change**.

GNU keeps arrays as an `ARRAY` object (`third_party/bash/array.h:30-41`) whose
length is a maintained field:

- `array.h:121 array_num_elements()` — `num_elements` is maintained by
  insert/delete, so `${#a[@]}` is O(1).
- `array.c:516 array_insert()` — appending an element is an O(1) list splice
  (`array.c:533-629` maintain `max_index`/`num_elements`).
- Sparse indices are legal: `a[100]=x` leaves `${#a[@]} == 1` and `${a[50]}`
  empty.

Rubash's projection is a rendered string in `ShellState.env_vars`, so a length
read had to split the whole render. Phase 1a adds the maintained count.

## Architecture decision

The design review draft (`PHASE1A_DESIGN_REVIEW.md`) proposed **candidate A**, a
new `HashMap<String, IndexedArrayValue>` side table keyed by name. Code
reconnaissance before implementation showed that the long-term form already
exists in the tree:

- `shell/state.rs:32-40` — `ShellState` already carries both
  `variables: VariableStore` (the typed owner, `variables.c shell_variables`)
  and `env_vars: VarTable` (the flat render mirror). The typed store is not new.
- `shell/variables.rs:10-14` — `ShellValue::{Scalar, IndexedArray(=
  BTreeMap<i64,String>), AssociativeArray}`.

Adding a side table would have created a **third** truth and a second thing to
invalidate. Phase 1a therefore rides the existing typed store and uses the
`IndexedArray` variant as the O(1) count source. `BTreeMap::len()` is already
O(1), so — as the review noted — no separate `num_elements` field is needed
(GNU maintains one only because a linked list has no `len`), and sorted
iteration reproduces GNU's ascending `${a[@]}` order for free.

The mirror is not removed. `env_vars` remains the projection every storage
reader sees; Phase 1a only adds a structured cell beside it.

### The invariant: complete or absent

> A structured cell is either a **complete** mirror of the same
> `indexed_array_entries` parse the render uses, or **absent**.

This is the rule that makes the cell safe to consult. Every write either syncs
the cell (`sync_typed_indexed_array` / `sync_typed_indexed_storage` /
`sync_typed_indexed_element`) or drops it (`drop_typed_indexed_array`). A
refused typed write (readonly) **drops** the cell rather than shadowing the
mirror — dropping only costs a fallback to the O(n) parse, while a stale cell
would return a wrong value.

- **Writes always sync.** Only reads consult the gate.
- **Reads** go through `array_length` (`arrays/executor.rs:373`), which returns
  the typed count when the cell is a complete `IndexedArray` and the name is not
  a nameref, else falls back to `array_values().len()` unchanged.
- `RUBASH_ARRAY_STORE=0` disables the structured **reads** (an escape hatch that
  cannot help per-miss: a wrong value must be globally bisectable).
- Associative arrays are out of Phase 1a scope; assoc typed cells are neither
  synced nor dropped by this change.
- Element writes use a single-key in-place extension
  (`VariableStore::extend_indexed_element`) when the cell is already complete,
  and verify the resulting size equals the caller's own parse size — so a cell
  that is not a complete mirror is never extended, it is replaced.

## Method

Measured 2026-10-15 on Termux/Android aarch64, 6 cores, kernel `6.16.x`,
Rust 1.99.0, release profile (opt-level 3, LTO). Reference shell: **GNU Bash
5.3.20** (`/data/data/com.termux/files/usr/bin/bash`).

> **Oracle caveat.** The repository's designated oracle is WSL GNU Bash 5.3.0
> (`AGENTS.md`); this host has only 5.3.20. Patch-level array semantics are not
> known to differ, but the baseline is 5.3.20, not 5.3.0, and no Windows/Linux
> x86_64 validation is claimed here.

Binaries compared:

- `master` — `~/.local/share/../usr/tmp/rb-master/target/release/rubash`
  (`6daf8e1`, separate git worktree, release).
- `branch` — `projects/rubash/target/release/rubash`
  (`perf/array-structured-store`, release).

Protocol: for each case, seven interleaved invocations per binary, best-of-7
taken (min filters transient host interference); each invocation has a 90 s
timeout and is reported `NA` when it trips. Workloads are files/one-liners from
`bench_niu/perf_battery/run_array_ab.sh`. Host load was **not** idle
(`loadavg ≈ 3.5–5.0` on 6 cores) because the agent harness itself is a large
CPU consumer, so these are best-of-N minima, not clean-room medians; ratios are
the claim, not absolute milliseconds.

## Results

| Workload | bash ms | master ms | branch ms | master/branch | verdict |
| --- | --- | --- | --- | --- | --- |
| `08_arlen` scalar control (200k) | 562 | 1601 | 1615 | 1.0x | unchanged (control) |
| `len@10k` + 200k `${#a[@]}` | 589 | **NA (>90 s)** | 2274 | — | 3.86x vs GNU, master unreachable |
| `len@1k` + 200k `${#a[@]}` | 583 | **NA (>90 s)** | 1685 | — | 2.89x vs GNU, master unreachable |
| `len@sparse` + 200k `${#a[@]}` | 588 | 1917 | 1641 | 1.2x | branch faster |
| `15_arrbuild` n=2000 | 30 | 2005 | 2008 | 1.0x | unchanged |
| `16_arriter` n=2000 | 42 | 1977 | 1998 | 1.0x | unchanged |
| `19_arrcopy` n=2000 | 5815 | 22790 | 19885 | 1.1x | branch faster |
| `comboundedit` (write+len) | 36 | 20422 | 10116 | 2.0x | branch faster |

Prior-art figure for the target construct: 10k-element array + 200k `${#a[@]}`
reads was `bash 594 ms`, `master >300 s` (timed out), `branch 2616 ms` — the
branch is the only one that completes, at 3.8x GNU.

### Regressing rows, preserved

An **intermediate revision of this branch** rebuilt the whole `BTreeMap` on
every element write and regressed the element-write battery 6–12% vs the
pre-#616 baseline (`15_arrbuild` 1943 → 2168 ms, `16_arriter` 1950 → 2181 ms,
`19_arrcopy` 22588 → 23880 ms). That regression is **fixed within the branch**
by the in-place single-key extension; the rows above are the post-fix
measurement and are unchanged (1.0x) or improved vs master.

### Not fixed by Phase 1a

`15_arrbuild` n=2000 remains 66x slower than GNU. Phase 1a deliberately does
not touch the per-write render, because the review scoped build cost to phase
1c. Note that the fixed cost is *per element write* — for n=2000 it is ~1 ms per
write, so the construct is **quadratic**, not a constant-factor gap. Closing it
requires a lazily materialised mirror (write the structured cell, defer the
render to readers that need the string), which is phase 1b/1c, not this PR.

## Semantic verification

- Library suite: **581/581**.
- `tests/array_store_parity.rs` (new, 34 GNU-verified cases): green with the
  store enabled **and** with `RUBASH_ARRAY_STORE=0`, covering indexed/sparse/negative
  subscripts, `${#a[@]}`/`${!a[@]}`/`${a[@]:1:2}`, `+=`, unset→reread, nameref
  write-through, `declare -p` round trip, temporary assignment, and the
  `mapfile`/`read -a`/`printf -v`/arith builtin writers.
- `cli_tests`: red set **name-identical** to master in the same tree (1
  pre-existing failure: `bashdb_compat::compound_assignment_splits_unquoted_associative_array_element`).
- `executor_tests`: failure set **identical** to master (1229 items, compared as
  a set because the bulk are pre-existing environment failures).
- Differential fuzz: 2500 generated array scripts (35 operations × 3–10 lines,
  byte-compared), **branch==master for all 2500**; both differ from GNU on the
  same 677 scripts (identical counts, pre-existing gaps, not introduced here).
- Three real defects were found and fixed by this discipline during the phase:
  (a) `unset 'a[1]'` left a stale cell (count 3 vs 2); (b) `((a[i]=v))` downgraded
  the typed cell to `Scalar`, then `printf -v a[i]` rebuilt it incomplete; (c) a
  readonly `read -a` wrote the mirror while the typed write was refused. All
  three are now covered by the parity suite.

## Acceptance

Phase 1a's target was `${#a[@]}` O(1) with zero behavioural change. Achieved:
the scalar control is unchanged, no workload regresses, length reads are
reachable where master times out, and every semantic gate matches master by
**set identity**, not by count. The build path remains quadratic and is
explicitly deferred; #616 is therefore **partially** addressed by this PR and
should be closed only together with the phase 1b/1c mirror work if the
maintainers want the build construct in scope.

These Termux/Android figures are not Windows or Linux x86_64 validation.
