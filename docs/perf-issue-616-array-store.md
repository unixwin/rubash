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

Protocol: for each case, five rounds; in each round all three shells run
consecutively and their order is **rotated** (`bash, master, branch` →
`master, branch, bash` → …), and the per-shell minimum over the rounds is
taken. Each invocation has a timeout and is reported `NA` when it trips.
Workloads are files/one-liners from
`bench_niu/perf_battery/run_array_ab.sh`. Host load was **not** idle
(`loadavg ≈ 3.5–5.0` on 6 cores) because the agent harness itself is a large
CPU consumer, so these are best-of-N minima, not clean-room medians; ratios are
the claim, not absolute milliseconds.

> **Why the order is rotated (a correction).** This host throttles, and the
> *first* case of a round is the fastest. Measured directly: the **same**
> binary ran **1.14x slower** merely for being invoked later in the round
> (`$PREFIX/tmp/order_ctl.sh`, same binary in positions 1 and 3). An earlier
> revision of this harness always ran `bash, master, branch`, so the `branch`
> was always last and accumulated a phantom ~10–15% penalty. The `19_arrcopy`
> row below is the visible casualty: it read `1.1x branch faster` under one
> ordering and `0.8x branch slower` under another, and is **1.0x** once the
> ordering is neutralised. Rows that move by less than the ~15% ordering
> amplitude should be read as parity.

## Results

Order-rotated best-of-5, release binaries, same tree and same minute. The
`master/branch` column is the ratchet: `< 1` would mean the branch regressed.

| Workload | bash ms | master ms | branch ms | master/branch | verdict |
| --- | --- | --- | --- | --- | --- |
| `08_arlen` scalar control (200k) | 659 | 1848 | 1886 | 1.0x | unchanged (control) |
| `build10k` compound + `${#a[@]}` | 39 | 79 | 79 | 1.0x | unchanged |
| `len@sparse` + 200k `${#a[@]}` | 669 | 2244 | 1906 | 1.2x | branch faster |
| `15_arrbuild` n=2000 | 33 | 2104 | 2130 | 1.0x | unchanged (1b takes this) |
| `15b_arrbuild` n=10000 | 55 | 57448 | 57984 | 1.0x | unchanged (1b takes this) |
| `15c_arrbuild_compound` n=2000 | 34 | 2318 | 2319 | 1.0x | compound append at parity |
| `16_arriter` n=2000 | 47 | 2319 | 2349 | 1.0x | unchanged (1b takes this) |
| `19_arrcopy` n=2000 | 6810 | 26592 | 27746 | 1.0x | unchanged |
| `comboundedit` (write+len) | 53 | 27210 | 11702 | 2.3x | branch faster |

No workload regresses. `comboundedit`'s 2.3x is Phase 1a's own win: it is
dominated by 2000 `${#a[@]}` reads, which stop parsing the render.

Marquee length-read construct — the fixed cost being removed (measured with the
**earlier non-rotated** protocol, which cannot flatter an `NA`):

| Workload | bash ms | master ms | branch ms | verdict |
| --- | --- | --- | --- | --- |
| 10k-element array + 200k `${#a[@]}` | 594 | **NA (>300 s)** | 2274 | 3.86x vs GNU; master unreachable |
| 1k-element array + 200k `${#a[@]}` | 583 | **NA (>90 s)** | 1685 | 2.89x vs GNU; master unreachable |

That is the deliverable: the branch is the only build that completes, at ~2.8–3.9x
GNU, where master does not finish at all.

### Regressions found and fixed within the branch

Two O(n)-per-write costs were introduced by intermediate revisions of this
branch and removed before it was proposed. Both were found by the same method —
measure every write path, not just the target read:

1. **Element write rebuilt the `BTreeMap`.** The first revision re-parsed the
   mirror and rebuilt the whole structured cell per `a[i]=v`. Fixed by the
   in-place single-key extension (`VariableStore::extend_indexed_element`),
   which is only taken when the resulting size equals the caller's own parse
   size, so a cell that is not a complete mirror is never extended. The
   element-write rows above are back to 1.0x.
2. **Compound append rebuilt the structured cell.** The next revision refreshed
   the cell from the whole parsed map on every write, so `a+=(...)` in a loop
   copied the entire array once per iteration — work the pre-#616 baseline never
   did. Fixed by `extend_indexed_array_from_parsed`: when the parsed map is a
   **verified** extension of the current cell, only the appended tail is copied.
   `15c_arrbuild_compound` is 1.0x above.

Both are the same lesson as the phase itself: the structured cell must be the
cheap thing, not a second full copy behind every write.

> The first revision's magnitudes (`15_arrbuild` 1943 → 2168 ms) were measured
> with the pre-rotation harness, whose ordering bias works *against* the branch,
> so they are an upper bound on a real effect rather than a clean figure. The
> mechanism — rebuilding n entries per write — is what was diagnosed and
> removed, and the post-fix rows are 1.0x.

### Not fixed by Phase 1a

`15_arrbuild` n=2000 is still ~64x slower than GNU (2130 ms vs 33 ms) and
`15b_arrbuild` n=10000 ~1050x. Phase 1a deliberately does not touch the
per-write render, and this cost is *per element write* — ~1 ms each at n=2000 —
so the construct is **quadratic**, not a constant-factor gap. `master` is 57 s
at n=10000 against the branch's 58 s, i.e. Phase 1a neither caused nor fixed it.

The element-write half is Phase 1b (stacked PR), which amortises `a[i]=v` to
O(log n) cell work plus an in-place mirror append. The *compound* half
(`a+=(...)`) still re-renders. What remains for both is a lazily materialised
mirror (write the cell, defer the render to readers that need the string),
which is Phase 1c.

## Semantic verification

- Library suite: **581/581**.
- `tests/array_store_parity.rs` (new, 38 GNU-verified cases): green with the
  store enabled **and** with `RUBASH_ARRAY_STORE=0`, covering indexed/sparse/negative
  subscripts, `${#a[@]}`/`${!a[@]}`/`${a[@]:1:2}`, `+=`, unset→reread, nameref
  write-through, `declare -p` round trip, temporary assignment, the
  `mapfile`/`read -a`/`printf -v`/arith builtin writers, and the
  incrementally-refreshed compound-append shapes (edit-then-append, sparse
  prefix, negative subscript).
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
**set identity**, not by count. The per-write render remains quadratic and is
explicitly deferred; #616 is therefore **partially** addressed by this PR.
Phase 1b (stacked PR, already implemented on top of this branch) removes the
element-write half; the compound/overwrite render and the lazily materialised
mirror remain Phase 1c.

These Termux/Android figures are not Windows or Linux x86_64 validation.

## Phase 1b — the element write is amortised (append path)

Phase 1a made `${#a[@]}` O(1) but left the element write quadratic, so it is
handled here as a continuation of the same concern: the structured cell is the
array's working truth, not just its counter.

### Attribution

`array_assignment_exec.rs` did **four** O(n) operations per `a[i]=v`:

1. `env_vars.get(name).cloned()` — deep-copies the whole mirror `String`;
2. `indexed_array_entries(&current)` — deep-copies the parsed map (128-entry
   string-keyed memo, so it is a cache *hit* and still O(n));
3. `format_indexed_array_storage(entries)` — re-renders the whole array;
4. `env_vars.insert(...)` — swaps in the new `String`.

So building an n-element array was O(n²). GNU touches one node —
`array.c:516 array_insert` splices one `ARRAY_ELEMENT` and maintains
`num_elements`/`max_index` (`array.c:533-629`, read by `array.h:121`) — and
never re-serialises anything, because the `ARRAY` object *is* the array.

### Fix

When the structured cell is a **complete** `IndexedArray`, the mirror is a
**canonical** `STORAGE_WORD_PREFIX(...)` render, and the write is an **append**
(`index == max_index + 1`, `0` for an empty cell), then:

- the old element is read from the cell (`BTreeMap::get`, O(log n));
- the cell gains one key (`extend_indexed_element`, O(log n));
- the mirror grows in place by exactly the segment the renderer would have
  appended (`append_indexed_mirror_element`, amortised O(1)).

Every other shape — overwrite, sparse insert, readonly, missing cell,
non-canonical mirror — falls through to the unchanged full path. This is a
whitelist whose fallback is the real path, not a symptom blacklist.

Two ordering details that matter:

- the mirror is extended **before** the cell, so if the cell write is refused
  the fallback re-parses the already-extended mirror and re-inserts the same
  element — idempotent, landing on the correct value rather than a stale one;
- "is the body empty?" is judged by **length**
  (`len == PREFIX.len() + 2`), never by `ends_with('(')`, because an element
  value may itself end in `(`. Getting this wrong made the first write into an
  empty array render as `( [0]=...)`; the differential vs master caught it in
  one line, and the regression case below fails if the wrong test is restored.

### Results

Order-rotated best-of-5, release binaries, same tree and same minute. The
`master/branch` column is the ratchet.

| Workload | bash ms | master ms | branch ms | master/branch |
| --- | --- | --- | --- | --- |
| `08_arlen` scalar control (200k) | 540 | 1842 | 1751 | 1.1x |
| `build10k` compound + `${#a[@]}` | 30 | 64 | 52 | 1.2x |
| `len@sparse` + 200k `${#a[@]}` | 582 | 1875 | 1610 | 1.2x |
| `15_arrbuild` n=2000 | 31 | 1760 | **51** | **34.5x** |
| `15b_arrbuild` n=10000 | 56 | 46071 | **157** | **293.4x** |
| `15c_arrbuild_compound` n=2000 | 29 | 1898 | 1915 | 1.0x |
| `16_arriter` n=2000 | 36 | 1930 | **64** | **30.2x** |
| `19_arrcopy` n=2000 | 5554 | 21843 | 22086 | 1.0x |
| `comboundedit` (write+len) | 42 | 23199 | **9890** | **2.3x** |

No row regresses, and the shape of the fix shows up in the scaling: `master`
goes 1760 → 46071 ms for 5x the elements (26.2x, **quadratic**), while the
branch goes 51 → 157 ms (3.1x, **~linear**). `15b_arrbuild` is still 2.8x GNU,
but it is no longer quadratic in the shell's own work.

`declare -p` byte-identity is the correctness bar for the in-place extension:
the mirror must equal a full re-render, and `declare -p` prints the mirror's
parse. `tests/array_store_parity.rs` pins it for the empty-array first write,
values ending in `(`/`)`, appends across a sparse gap, and compound-then-element
append — each also re-run with `RUBASH_ARRAY_STORE=0`.

### Not addressed here

- The **compound** append `a+=(...)` still re-renders the whole mirror
  (`append_array_value`, `arrays.rs:779`, opens with `indexed_array_entries`
  and the caller re-renders), so `a+=($i)` in a loop is quadratic **in the
  render**. The structured cell is no longer the problem: Phase 1a already
  refreshes it tail-only with a verified prefix
  (`extend_indexed_array_from_parsed`, O(tail·log n)), which is what took the
  compound battery back to 1.00x. Closing the remaining render cost means an
  incremental *insert into the render* — the natural Phase 1c, on top of this
  cell.
- A mid-array overwrite (`a[3]=x` in a loop) is still O(n) per write, because
  the render is text and patching a middle token needs a byte-offset index.
- The root fix for both is a **lazily materialised mirror**: write the cell and
  defer the render until a reader that needs the string asks for it. That needs
  a materialisation point at every whole-table consumer — `env_vars.iter()` (2),
  `env_vars.clone()` (18), `values_map()` (4), whole-table for/`into_iter` (2),
  plus 5 child-environment construction sites — as well as the 60
  `parameter_array_storage` and 82
  `array_values`/`array_value_at`/`array_indices` call sites. It is a design of
  its own (Phase 1c), not a patch to land on top of this PR.
