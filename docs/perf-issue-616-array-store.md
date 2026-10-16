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

## Phase 1c — the compound append grows in place

Phase 1b's "Not addressed here" named this one: `a+=(...)` still re-parsed and
re-rendered the whole mirror, so the loop idiom `a+=($i)` stayed quadratic. It
is the same concern as 1a/1b — the structured cell is the array's working
truth, not a counter — so it is Phase 1c rather than a new issue.

1c takes the *seed-and-patch* half of the design 1b sketched: decode from the
cell, then patch the canonical render in place. The full lazily-materialised
mirror (defer the render at every whole-table consumer) is left for later; the
render is still there, it is just no longer rebuilt per element.

### Attribution

The chain for `a+=( ... )` (all citations are the vendored GNU bash 5.3 tree,
`third_party/bash/`):

1. `expand_compound_array_assignment` (`arrayfunc.c:557`) →
   `assign_compound_array_list` (`arrayfunc.c:700`).
2. `last_ind = array_max_index(a) + 1` (`arrayfunc.c:743`) — GNU reads the
   `ARRAY` object's shape (`array.h:30-41`), inserts each element
   (`array_insert`, `array.c:516`) **in place**, and never rebuilds the array
   from its serialised form.
3. rubash's `append_array_value` (`src/executor/arrays.rs`) instead opened with
   `indexed_array_entries(current)` — a full parse of the rendered mirror into
   a `BTreeMap` — and the caller then re-rendered the whole mirror
   (`format_indexed_array_storage`) and re-synced the cell. Three O(n) passes
   per element appended.

The parse is memoised (`indexed_array_entries_cached`, `arrays/storage.rs:64`:
content-addressed on the exact storage string, 128 entries, "a stale parse
cannot be served"), and each render feeds the memo back — so in master it is a
*hit*. It is still `as_ref().clone()` of the whole map (`storage.rs:51`), i.e.
O(n) allocations, which is why 1b's element write could be amortised but the
compound append could not.

Measured on untouched master (`37d6caf`), n elements in a loop, release build:
`cmp2000` 1985 ms, `cmp4000` 8175 ms — 4.12x for 2x the elements
(**quadratic**), and 50.9x / 190.1x the GNU time (39 ms / 43 ms) for the same
work.

### Fix

GNU's `ARRAY` object *is* rubash's structured cell. Phase 1a established
"the structured cell is a COMPLETE mirror of the render, or it is absent";
1b already trusts that invariant for the element write. 1c seeds the compound
scan from the same cell:

- `Variables::indexed_array_cell` (`src/shell/variables.rs`) exposes the cell —
  the read GNU makes through `array_max_index` (`arrayfunc.c:743`).
- `IndexedDecodeBase` (`src/executor/arrays.rs`) is the decode view: it borrows
  the cell, keeps this call's writes in a small `mutations` map, and answers
  `max` / `get` / `is_empty` in O(log n) **without ever building the element
  map**. It materialises the full map only when a render is actually needed.
- `append_array_value` takes the cell and returns
  `IndexedAppendOutcome { entries: Option<_>, old_max, appended, appended_only }`.
  For a pure append (`appended_only`) `entries` is `None`: the write is fully
  described by `appended`, the elements strictly above `old_max`. The two reads
  that used to need the map were re-pointed at O(1) state —
  `array_assignment_index` now takes the max index (it only ever used
  `keys().next_back()`) and the `[i]+=` read goes through the view.
- On that route the executor grows the mirror in place by exactly the segments
  the renderer would have appended (`append_indexed_mirror_segments` — 1b's
  per-element helper generalised to a segment list, which is now the single
  owner of both) and extends the cell (`extend_indexed_element`). No render, no
  re-parse.

Every other shape — a subscript at or below `old_max` (an overwrite), an integer
array (the post-pass rewrites every element, so `appended_only` is false by
construction), no cell, a non-canonical mirror, a readonly target — falls back
to `render_indexed_outcome`, which rebuilds the map from the mirror and folds
`appended` back in. That fallback is the unchanged real path: a whitelist whose
fallback is the real thing, not a symptom blacklist.

Ordering is inherited from 1b and still matters: the mirror is extended
**before** the cell, so a refused cell write re-renders and re-syncs from an
already-correct mirror; and "is the body empty?" is judged by **length**, never
by `ends_with('(')`.

### An intermediate cut that measured SLOWER (kept for the record)

The first attempt removed the render and the trailing sync but still seeded from
`indexed_array_entries(current)`. Because the in-place route no longer renders,
it no longer feeds the parse memo — so every iteration became a **miss**, an
uncached O(n) parse replacing a memo-hit O(n) clone. The order-rotated A/B said
so plainly, and the measurement is why the design changed rather than being
tuned by eye:

| case | bash | master | branch (step 1) | master/branch |
| --- | --- | --- | --- | --- |
| `cmp2000` | 42 | 2453 | 3649 | **0.67x** |
| `cmp4000` | 51 | 9896 | 14277 | **0.69x** |
| `cmp_literal2000` | 42 | 2397 | 3565 | **0.67x** |
| `elem2000_ctl` (1b control) | 37 | 68 | 71 | 0.96x |

Removing work is not the same as removing the *right* work; only seeding from
the cell removes the O(n) itself.

### Results

Order-rotated best-of-9, release binaries, same tree, same host; `master/branch
> 1` means the branch wins. Shell order is rotated per round because a binary
invoked later in a round reads measurably slower here.

| case | bash ms | master ms | branch ms | master/bash | branch/bash | master/branch |
| --- | --- | --- | --- | --- | --- | --- |
| `cmp2000` | 39 | 1985 | **68** | 50.9x | 1.74x | **29.2x** |
| `cmp4000` | 43 | 8175 | **95** | 190.1x | 2.21x | **86.1x** |
| `cmp_literal2000` | 34 | 1979 | **63** | 58.2x | 1.85x | **31.4x** |
| `elem2000_ctl` (1b control) | 33 | 61 | 55 | 1.85x | 1.67x | 1.11x |

The control row (1b's element write) does not regress, so the win is entirely in
the compound path. The shape is the point: master goes 1985 → 8175 ms for 2x the
elements (4.12x, quadratic) while the branch goes 68 → 95 ms (1.40x, ~linear).
`cmp2000` is now 1.7x GNU instead of 50.9x, and the remaining gap is the render
that 1c still performs once per call.

The master side drifts with host load (this host is never idle — `cmp2000` read
2361 ms in one clean run and 1985 ms in another), which is why the *ratio*,
measured in the same tree with per-round shell-order rotation, is the claim
here and not the millisecond.

### Semantic verification

- `tests/array_store_parity.rs` green with the store on and with
  `RUBASH_ARRAY_STORE=0`; Phase 1c cases added: loop append, batch twice, sparse
  start, sparse gap, rewrite-then-append (fallback), empty list, unset target, a
  value ending in `(`, integer array (fallback), interleaved element writes,
  quoted word with a space, expansions.
- 2500-seed differential fuzz (branch vs master vs GNU bash) over the write
  routes that can precede an append — `unset`, `readonly`, `declare -i`,
  `mapfile -O`, `local`, namerefs, `export`, `eval`, `compgen` — plus 1c
  compound-append routes; the reported signal is `branch != master`.
  Result: **0/2500**, and `bash != branch` (622) equals `bash != master` (622),
  so no new GNU divergence was introduced.
- Whole-class differential matrix (`~/bench_niu/compound_matrix.sh`): 52 cases
  across assignment position × literal/variable/substitution/arithmetic ×
  spacing/quoting × append shape × attributes. Result: **0** `branch != master`
  (the single `bash != master` row is the pre-existing `bash:` vs full-path
  diagnostic prefix, identical before and after).
- `cli_tests` and `executor_tests` failure **sets** identical to master `37d6caf`
  (compared serial, `--test-threads=1`): `cli_tests` 1 vs 1 with no row on one
  side only, `executor_tests` 0 new failures (1172 vs 1173; the single
  master-only row passes on the branch and is a flake). Run in parallel this
  suite is not a usable gate: at `--test-threads=4` it races on shared paths
  under `target/` (`target/rubash-lineno-assignment-output.txt: No such file or
  directory`) and reported 2 "new" `cli_tests` failures and 4 new / 6
  disappeared `executor_tests` rows, every one of which passes in isolation.
  The comparison is therefore serial, and by failure set, never by count.
- `cargo test --lib` 584/584.

### Found while building the 1c matrix, filed separately

An out-of-range negative subscript in a compound assignment (`a=(); a+=([-1]=x)`,
`a=(p); a+=([-2]=Q)`) is silently dropped by rubash, where GNU reports
`bad array subscript` (`arrayfunc.c:801-809`), abandons the assignment and
(non-interactive) exits 1. Pre-existing on master, unrelated to the store, and
filed as its own bug — #631 — so this performance line stays one concern.

### #631 resolved (correctness of the compound element-header scan)

Fixed by the #631 PR for the **assignment** route. The wrong invariant was
`array_assignment_index` (`src/executor/arrays.rs`): it returned
`Option<usize>`, conflating "this word is not a `[subscript]` header" with
"this subscript resolved out of range", so the scan's
`if array_assignment_has_subscript(left) { continue; }` dropped the element in
silence — for both element forms (`[sub]=`, `[sub]+=`) and both compound forms
(`a+=(...)`, `a=(...)`).

GNU's contract, from the vendored 5.3 source: `assign_compound_array_list`
(`arrayfunc.c:700`) counts a negative subscript back from the end
(`array_max_index + 1 + ind`, `:802-803`); when it is still below zero it calls
`err_badarraysub (w)` on the whole element word — through `report_error`
(`error.c:440`; message `arrayfunc.c:63`, so **no** `command:` segment, unlike
`declare: x: readonly variable`) — counts `any_failed` and `break`s the list
(`:804-809`). Elements inserted before the failure stay stored (`:768-772`),
and `assign_array_var_from_string` (`:913-932`) returns NULL, so the assignment
fails: `bash_variable_assignment_error` (`subst.c:11169`) →
`exp_jump_to_top_level (DISCARD)` (`subst.c:11174`).

The fix returns a three-state `ArrayAssignmentIndex { Index, NegativeOutOfRange,
Unresolved }`, stops the scan on `NegativeOutOfRange` with the word carried in
`IndexedAppendOutcome::bad_subscript`, stores the partial outcome, then reports
and fails with the same jump the plain `a[-2]=q` path already makes. Because the
jump is the plain path's, the compound and plain routes stay consistent; the
remaining `eval`-containment divergence is #637.

Evidence: 43-case 3-way differential vs GNU (`~/bench_niu/matrix_631.sh`) —
25 fixed, **0 regressions**, 6 pre-existing divergences unchanged;
`~/bench_niu/verify_631.sh` for the per-route byte comparison;
`tests/issue631_neg_subscript_regressions.rs`; failure-set ratchet vs `81c0df5`
(`~/bench_niu/ratchet.sh`, serial) clean.

Sibling divergences found while characterising the class, each its own concern:
And issue #635 (the declare-side duplicate of this scan, `declare`/`local`/`typeset`, still
drops the element — it needs the builtin's own fatal error channel), #636
(`a+=( [@]=q )` reports an arithmetic error instead of GNU's `cannot assign to
non-numeric index`, `arrayfunc.c:787-792`), #637 (`eval` does not contain the
DISCARD jump; reproducible on master with the plain `eval 'a[-2]=q'`).

Measurement caveat for the transportable numbers above: the host has no idle
window (loadavg ~3.5–5 on 6 cores) and its local bash is 5.3.20, while the
repository oracle is WSL bash 5.3.0. Every rule applied is taken from the
vendored 5.3 source and its own probes, not from a version's observed output.

## The assoc cell and `set -u` boundness (#638; attribution for #618)

### Attribution — why `m[k]=v` is ~18x GNU

Issue #618 reports the assoc write path at 16.7x. Re-measured on `12da0b5` with the
order-rotated min-of-3 harness `~/bench_niu/attrib_618.sh` (bodies inlined into
`-c` so neither shell pays an `eval` the other does not; loadavg 5–8 on 6
cores, so min-of-rotated-rounds rather than a single run):

| case | GNU | rubash | ratio |
| --- | --- | --- | --- |
| `m[k$((i%100))]=$i`, N=200000 | 0.921s | 16.830s | 18.3x |
| same with 1 key | 0.712s | 2.495s | 3.5x |
| `m+=( [k…]=$i )`, N/20 | 0.071s | 0.163s | 2.3x |
| assoc read, 100 keys | 1.279s | 9.076s | 7.1x |
| indexed write, 100 slots | 0.799s | 20.424s | 25.6x |
| scalar write | 0.603s | 1.301s | 2.2x |

The mechanism is isolated by scaling the **key count** K with the write count
held at 20 000: the per-write cost tracks the number of keys, because every
write re-renders the whole mirror (`assignment_helpers.rs:345
format_assoc_storage`), while GNU's `assoc_insert` is an O(1) hash insert
(`assoc.c`).

| K | GNU | rubash | ratio |
| --- | --- | --- | --- |
| 10 | 0.007s | 0.008s | 1.1x |
| 100 | 0.007s | 0.013s | 1.9x |
| 1000 | 0.011s | 0.242s | 22x |
| 5000 | 0.025s | 5.574s | 223x |

The indexed side shows the same shape far more sharply (K=5000: GNU 0.109s vs
rubash 98.947s = 908x); that residual is #616's next phase, not #618's.

### #638 resolved — the scalar form is bound by element 0, and the mirror decides

Surveying the assoc cell for #618 exposed two defects behind one symptom:
`set -u` judged the **scalar form** `${m}` wrong in *both* directions —
`declare -A m=([k]=v); unset "m[k]"` printed `x[]` at status 0 where GNU aborts
at 127, and `declare -A m; m[0]=v` aborted at 127 where GNU prints `[v]`.

GNU decides from element/key `"0"`:

```c
/* subst.c:7787-7790 parameter_brace_expand_word() */
else if (assoc_p (var))  temp = assoc_reference (assoc_cell (var), "0");
else if (array_p (var))  temp = array_reference (array_cell (var), 0);
/* array.c:657 / assoc.c:120 return NULL when that element is absent;
   subst.c:10162-10169 parameter_brace_expand() then raises err_unboundvar
   (error.c:448), with all_element_arrayref exempting `${a[@]}`. */
```

rubash's rule was `!entries.is_empty()` — "has any element" — wrong for
`IndexedArray` and `AssociativeArray` alike. Worse, for assoc it consulted the
typed cell, which **no writer maintains**: only `declare`/`typeset`
(`builtins/declare.rs:105`, `:193`) and `printf -v m[k]` (`builtins/printf.rs:365`)
create it, while every later mirror write (`m[k]=v`, `unset "m[k]"`, `m=(...)`,
`m+=(...)`, `${!name[k]}`, `wait`-bound assoc variables, `export`/`readonly`)
touches only the mirror string. The indexed side has a sync family for exactly
this (`arrays/storage.rs:390 sync_typed_indexed_storage`, `:321
sync_typed_indexed_element`); there is **no assoc counterpart**. Since the
mirror is what the *value* read uses (`variable_state.rs:462-470
scalar_parameter_value` → `assoc_value_at(storage,"0")` / `array_value_at(storage,0)`),
the stale cell let boundness disagree with the value it was guarding.

The fix changes the invariant rather than chasing the ~12 writers: **the
mirror decides**, through those same two helpers, so boundness and value agree
by construction and a future writer cannot silently reintroduce the class. The
typed cell is still consulted for scalars (authoritative by construction) and
for unmarked arrays, and the array rule becomes `contains_key(0)` /
`contains_key("0")`. Both parses are content-addressed memos
(`arrays/storage.rs:64`, `assignment_helpers.rs:491`) and the value read
performs the same one, so this costs a cache hit, not a re-parse — the indexed
`set -u` fast path keeps its O(1) shape.

Evidence: 40-case GNU differential (`39 match`; the 1 remaining diff is
The remaining diff is issue #639 — an arithmetic *subscript* never runs the nounset check, which fails
during the assignment in GNU), `tests/issue638_nounset_array_boundness.rs`
20/20, `cargo test --lib` 584/584, serial failure-set ratchet vs `12da0b5`.

Sibling divergences found while probing the class, each its own concern:
And issue #639 (arithmetic array subscripts ignore `set -u`; the `for ((;;))` header also
reports status 1 where GNU reports 127).

Measurement caveat as before: local bash is 5.3.20, the repository oracle is
WSL bash 5.3.0, and every rule applied here is taken from the vendored 5.3 C
source cited above rather than from an observed version's output.

## #618 perf phase — the element write no longer re-renders (append + compact)

### Attribution — three O(keys) steps, not one

Decomposing one `m[k]=v` at K=5000 (crate-internal microbench, since
superseded and removed): string clone 1.47µs, parse+memo-hit+**Vec deep-clone
out of the Arc** 715.69µs, full re-render 2139.84µs. The earlier parse memo
only removed the *scan*; the clone (24–25%) and the render (73–75%) stayed
O(keys). Fixing only the render could never flatten the curve.

A cell-authoritative + lazy-render redesign was surveyed and **rejected**: the
read surface is ~125 A-class sites behind one good hook
(`parameter_array_storage`, 54 callers, owned `String`) but ~20 direct
`env_vars.get(name)` borrows through `VarTable`'s `Deref<HashMap>` admit no
hook, and 8 B-class groups need the literal bytes (`set` listing, scope
save/restore round-trips, two unfiltered OS-env projections). Routing all of
them is #638's dual-store class at 20× the blast radius.

### Fix

Plain `m[k]=v` (`!append`, integer attribute pre-evaluated as before)
appends one `[k]=v` segment before the closing `)` in O(segment) instead of
parsing + re-rendering (`array_assignment_exec.rs`, `assignment_helpers.rs:
assoc_element_segment`, `assoc_storage_append`). The segment is quoted by the
same quoters the full renderer uses, so an append-built string is
byte-identical to a rendered one whenever no key repeats. `+=`,
non-`(...)` storage, and append-failures fall through to the unchanged
canonical path (whitelist with the real path as fallback). A per-variable
`assoc_compact_budget` (`shell_state.rs`, seeded from the canonical entry
count, cloned with the state) buys O(entries) appends per O(entries)
compaction — amortized O(1) per write, string never beyond twice canonical —
and is scheduling-only: a stale budget moves the compaction, never the value.

Appending creates transient duplicated segments, so three readers had to agree
on the contract `bash_assoc_order` already documents — *"first occurrence
fixes the slot; last occurrence supplies the value"* (GNU `assoc.c
assoc_insert` overwrites in place, one binding per key):

- `assoc_entries_uncached` + builtins `parse_assoc_words` + printf's own
  parser fold at the source via one shared `dedupe_assoc_entries`
  (first-slot, last-value; identity on dupe-free input, i.e. a no-op on the
  pre-existing corpus). Every RMW (`unset` retain, forward-`find` scalar
  writers, `rev`-`find` element writers) is then safe regardless of find
  direction.
- `bash_assoc_order` now implements what its comment promised: the slot keeps
  its first index, the value comes from the key's last index. Previously it
  returned the first value — unreachable before (no writer ever produced
  dupes), exposed by a direct injection probe.
- `${#m[@]}` counts `assoc_entries(...).len()` for assocs instead of raw
  `array_values` segments.

### The tokenizer was O(n^2) — found by the miss, fixed at the root

Compaction parses a novel (never-memo-fed) dupe-laden string, and the true
parse measured ~O(n^1.6): 201 segments 1.0ms, 2001 24.6ms, 10001 563ms.
Root cause: `StorageWordIter::next()` collected the **entire rest of input**
into a `Vec<char>` for every `[`-leading word (i.e. every assoc segment)
just to measure the subscript span — O(remaining) per word. Fixed with
`lexer::skip::arraysub_span_len_str`, the identical state machine over `&str`
returning a byte length (same GNU owner, parse.y:5635-5651 P_ARRAYSUB), used
by the two storage-input splitters (executor `assignment_helpers.rs`, declare
`storage/words.rs`). Five other `arraysub_span_len` callers take
command-bounded single tokens and are untouched. True parse after the fix:
10001 segments 17.8ms, linear.

### Results — the curve is flat

Key-count sweep, N=20000 writes, min-of-3 order-rotated (same tree, release):

| keys | master/write | branch/write | master/GNU | branch/GNU |
| --- | --- | --- | --- | --- |
| 1 | 44.75µs | 45.90µs | 8.77x | 9.00x |
| 100 | 87.45µs | 47.85µs | 15.08x | 8.25x |
| 1000 | 537.40µs | 48.90µs | 99.52x | 9.06x |
| 5000 | 2361.35µs | 52.40µs | 445.54x | 9.89x |

5000x key growth moves branch per-write 1.14x (host noise); master blows up
52.8x. K=5000 branch is 45x faster than master; the GNU gap closes
445x → 9.9x.

Residuals, honestly: K≤3 branch trails master ~+1.5µs/write (~3%). Rate-fit
over K∈{1,2,3,5,10} (canonical share ≈1/(K+1)) attributes it structurally —
master's canonical always memo-HITs (it parses the string it just fed) while a
compaction parse always memo-MISSes (append-built strings are novel by
construction); the miss premium is linear and 2x-bounded after the tokenizer
fix. It vanishes by K=5 and is negative beyond. The remaining ~9x vs GNU is
fixed per-statement pipeline overhead (present at K=1, shared with scalar
writes) — a different subsystem, not this issue.

### Verification (this phase)

- 50-case assoc-write GNU differential battery: 50/50 identical before and
  after (master baseline 50/50, so the battery is a clean guard).
- Focused targets green: `array_store_parity`, `issue371`, `issue638`,
  `array_compound_quote_regressions`, declare/nameref/export suites.
  (`issue326/unset_tz_uses_system_localtime_offset` fails identically on
  unmodified master — Android tzdata environment issue.)
- Full serial failure-set gate vs `ce471a2` + niubash product A/B: see the
  PR record.

## #616 overwrite arm — append replaces re-render for `a[i]=v` too

### Attribution

Key-count sweep on `333973c` (N=20000 `a[i%K]=$i`, min-of-3 order-rotated,
release; new harness `~/bench_niu/sweep_616_keys.sh`, no indexed sweep
existed before): per-write 14.80µs (K=1) → 3571µs (K=5000), 736x vs GNU at
the top. Same three O(n) steps as assoc (mirror clone, BTreeMap deep-clone
out of the memo, full re-render). The tail-append fast path (1b/1c) already
covered `index == max+1`; everything else — overwrites, sparse jumps —
paid full price, which is exactly what a hot loop does after the fill.

### Precondition: the declare-seeded dense cell

The 50-case indexed differential battery (new, `~/bench_niu/indexed_diff.sh`)
caught a live correctness bug on master before any perf change:
`declare -a a=([5]=x [2]=y); a[2]=z` printed
`([2]="y" [5]="x" [2]="z")` with count 3. Bisected to
`builtins/declare.rs sync_typed_assignments`: it mapped entries to
values-only and dense-replaced, seeding `{0,1}`, so the 1b fast path fired
on `index == max+1 == 2` and appended. (Bare `a=([5]=x [2]=y)` seeds
correctly through the executor compound tail; only the declare-builtin
route dropped keys.) Both declare sync sites now preserve keys via
`replace_indexed_array_entries` — the helper's own doc comment already named
this exact case. The 1b path (and the new arm) trust the cell, so an
untrustworthy cell is a correctness precondition of the perf fix, fixed in
the same PR like `bash_assoc_order` was for #618. The `unset "a[@]"`
stale-count sibling lives on the unset path and was filed as #651, not
bundled.

### Fix

New overwrite/sparse-jump arm beside 1b in `execute_array_element_assignment`:
with a complete cell, a canonical mirror, and budget remaining, append one
`[i]=v` segment (O(segment)) and extend the cell (overwrite → same length,
new index → len+1, decided by a cell lookup; `extend` verifies the size and
falls back otherwise). `+=` reads the old value from the cell, so it parses
nothing either. Mirror-first-cell-second like 1b, so the canonical fallback
re-parses the extended mirror and self-heals. Budget map renamed
`assoc_compact_budget` → `array_compact_budget` and shared; canonical renders
re-seed from the (already distinct) BTreeMap length. No parse/render changes:
indexed readers fold by construction. The two verbatim
`format_array_value` printers (`declare -p`, `export -p`) re-render from the
folded map only when segments outnumber distinct keys (new shared
`fold_indexed_storage_dupes`; identity otherwise, prefix stripped for print
shape).

### Results

| keys | master/write | branch/write | master/GNU | branch/GNU |
| --- | --- | --- | --- | --- |
| 1 | 17.10µs | 18.75µs | 3.60x | 3.95x |
| 100 | 101.10µs | 20.85µs | 21.51x | 4.44x |
| 1000 | 889.55µs | 20.45µs | 172.73x | 3.97x |
| 5000 | 3638.45µs | 20.60µs | 673.79x | 3.81x |

177x faster than master at K=5000; GNU gap 674x → 3.8x. The indexed
splitter (`rendered_array_parts`) shows no superlinearity — the flat curve
with compactions included is the proof. Residuals: K=1 +1.6µs structural
memo-miss price (same as #618, negative by K=10); ~3.8x fixed pipeline
overhead (separate subsystem).

### Verification (this phase)

- Indexed battery 49/50 (only #651); master baseline 48/50.
- Focused suites green; the two failures fail identically on baseline
  (Android tzdata + positionals environment issues).
- Full serial failure-set gate vs `333973c` + niubash product A/B: see the
  PR record.

## #639 — nounset in arithmetic subscripts on the write path (split from #638)

`set -u; a[k]=v` silently stored with status 0. GNU evaluates every
subscript through `evalexp` → `expr_streval`, whose unbound block aborts on
both read and write (`expr.c:1190-1216`). The write-path subscript
evaluator was missing the nounset prologue its arithmetic
command/expansion siblings have.

The trap is FORCE_EOF, not DISCARD: a distinct `nounset_force_eof` cell
rides the shared abandon vehicle, and the ast abandon point recomputes the
context status there (127 at `-c`, 1 in script) — verified matrix on both.
Every boundary that clears the pending flag clears the cell alongside
(subshell/comsub/external/function resets). Compound `m=([k]=v)` needed one
more hop (its failure maps through `apply_shell_assignment` to
`ExpansionFailure(1)`; the cell now diverts to `Err(ExitCode(fatal))`).
`unset` on a missing array stays silent — GNU evaluates subscripts only
when the array exists (probed). No memo changes: subscript memo frames are
per-word-expansion (both toggle directions probed). Battery 10/25 → 25/25;
regression file `tests/issue639_nounset_subscript.rs`.

## #637 — eval contains the assignment DISCARD jump (split from #631)

`eval 'a=(p); a[-2]=q; echo IN'; echo AFTER` ended the whole script.
GNU catches DISCARD at the eval boundary (`evalstring.c:365,422-442`):
eval returns failure and the caller continues. Rubash absorbed
`ExpansionFailure` there but let the `evalerror_pending` abort escape.

A distinct `evalerror_subscript_pending` cell marks bad-subscript aborts
(armed at the element-write sites + compound tail); the eval boundary
absorbs exactly this class on `Ok`, leaving exit status to the last
command run (GNU parse_and_execute loop semantics: a later line that ran
fine yields eval rc 0). Arithmetic and nounset failures never arm it and
keep escaping. The eval'd top list runs at a recorded depth so nested
aborts fall through to line-skip logic instead of unwinding silently;
function/loop bodies inside eval run deeper and keep unwinding. Consumed
at the abandon point (except inside eval strings) and cleared at every
pending-clearing boundary. Battery 9/20 -> 20/20; regression file
`tests/issue637_eval_discard.rs`.

## #635 — abort declare/local on out-of-range negative subscripts

`declare -a a=( [-2]=q )` printed AFTER with status 0. GNU reports the
whole element word and fails without running on (arrayfunc.c:804-809).
The builtin resolver conflated "no header" with "out of range"; it now
resolves 3 states and breaks with partial store kept, and the word
travels up via out-params for the fatal diagnostic + DISCARD abort at the
Executor layer (mirroring the arith_eval_failure precedent, bare
report_error shape). Eval containment reuses the #637 absorption
primitive. export/readonly record failure status with the same
diagnostic; their list-abort threading is a documented follow-up.
Battery 6/20 -> 20/20; regression file
`tests/issue635_builtin_oob.rs`.
