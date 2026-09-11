> **ARCHIVED** — This document is historical. For current development practices,
> see `AGENTS.md`. Last current: 2026-08-10.

# Rubash Performance Debugging Process

This document records the concrete process used to diagnose and optimize the
Rubash executor hot paths during the performance work that landed through PRs
#10, #11, #12, and #13. It is intentionally procedural: the goal is to make the
next performance pass repeatable instead of relying on memory.

## Goal

The near-term target was not a broad benchmark suite. We focused on a small set
of tight shell workloads that exposed interpreter overhead:

- `loop.sh`: nested loop/control-flow overhead.
- `arith.sh`: arithmetic expansion and assignment overhead.
- `function-noop.sh`: function dispatch overhead with very little body work.
- `function-args-arith.sh`: function dispatch plus positional arguments and
  arithmetic expansion.

Each workload lives under the local benchmark scratch directory:

```text
C:\Users\caomengxuan\repo\tmp\rubash-perf
```

The comparison runners were:

```text
C:\Users\caomengxuan\repo\wt-rubash-baseline\target\release\rubash.exe
C:\Users\caomengxuan\repo\wt-rubash-perf\target\release\rubash.exe
C:\Program Files\Git\bin\bash.exe
```

The measurement rule was: run each case repeatedly, compare medians, and only
keep changes that produced a visible win without adding compatibility failures.

## Repository Hygiene

The main `rubash/` and `winuxsh/` directories often had unrelated local changes.
Those were treated as user work and were not modified directly.

Instead, performance work used clean worktrees:

```text
C:\Users\caomengxuan\repo\wt-rubash-baseline
C:\Users\caomengxuan\repo\wt-rubash-perf
```

Before each round:

```sh
git fetch origin --prune
git -C C:\Users\caomengxuan\repo\wt-rubash-baseline reset --hard origin/master
git -C C:\Users\caomengxuan\repo\wt-rubash-perf status --short --branch
```

This made the baseline/current comparison meaningful and avoided mixing
performance patches with unrelated local work.

## Benchmark Harness

The ad-hoc benchmark harness used PowerShell `Measure-Command` with a warmup and
median aggregation. A typical run looked like this:

```powershell
$cases = @(
  @{ name='loop'; script='C:\Users\caomengxuan\repo\tmp\rubash-perf\loop.sh' },
  @{ name='arith'; script='C:\Users\caomengxuan\repo\tmp\rubash-perf\arith.sh' },
  @{ name='function-noop'; script='C:\Users\caomengxuan\repo\tmp\rubash-perf\function-noop.sh' },
  @{ name='function-args-arith'; script='C:\Users\caomengxuan\repo\tmp\rubash-perf\function-args-arith.sh' }
)

$bins = @(
  @{ name='baseline'; exe='C:\Users\caomengxuan\repo\wt-rubash-baseline\target\release\rubash.exe' },
  @{ name='current'; exe='C:\Users\caomengxuan\repo\wt-rubash-perf\target\release\rubash.exe' },
  @{ name='git-bash'; exe='C:\Program Files\Git\bin\bash.exe' }
)
```

The important discipline was to compare both:

- current Rubash vs `origin/master`;
- current Rubash vs Git Bash.

The first comparison showed whether a patch helped Rubash. The second showed
whether we were actually closing the Bash gap.

## Profiler Setup

We first tried Windows Performance Recorder because it was installed:

```powershell
wpr -status
wpr -start CPU -filemode
wpr -stop target.etl
```

That path was blocked by Windows policy:

```text
0xc5585011 Failed to enable the policy to profile system performance
```

Because the current tool environment could not elevate through UAC, the workflow
switched to `samply`, which was able to collect user-mode samples without that
policy change:

```powershell
$env:CARGO_PROFILE_RELEASE_DEBUG='2'
$env:RUSTFLAGS='-C force-frame-pointers=yes'
cargo build --release --locked

samply record `
  --save-only `
  --keep-etl `
  --main-thread-only `
  --unstable-presymbolicate `
  --symbol-dir target\release `
  --iteration-count 20 `
  -o .tmp\samply-function-args-arith.json.gz `
  -- .\target\release\rubash.exe C:\Users\caomengxuan\repo\tmp\rubash-perf\function-args-arith.sh
```

The profile output was a gzipped Firefox/Mozilla profile JSON. When names were
not fully symbolicated, addresses were mapped with `llvm-symbolizer`:

```powershell
llvm-symbolizer -e target\release\rubash.exe 0x140237d46
```

The useful pattern was:

1. Build with release optimizations plus debug symbols.
2. Record a focused workload, usually `function-args-arith.sh` or
   `function-noop.sh`.
3. Summarize inclusive/exclusive samples from the JSON.
4. Symbolize the top addresses.
5. Patch one hot path.
6. Rebuild, run focused tests, run broad tests, then benchmark.

## Hotspots Found

The initial sampled profiles showed the cost was not in external process
execution. It was mostly executor bookkeeping:

- repeated Windows process environment writes through `env::set_var`;
- assignment expansion doing unnecessary HOME/USERPROFILE lookup;
- `_` being synchronized through the process environment after every command;
- function call bookkeeping materializing Bash dynamic arrays every call;
- process-substitution preparation cloning or materializing command structures
  even for simple commands that could not use process substitution;
- `PIPESTATUS` being stored as a formatted indexed-array string every command;
- command word expansion cloning full `CommandNode` structures;
- alias post-processing cloning expanded command nodes even when no aliases
  existed.

The recurring lesson was that Bash compatibility state should often be an
internal lazy view, not an eagerly formatted shell variable string.

## Optimization Rounds

### 1. Loop AST Reuse

PR #10 reduced loop overhead by reusing parsed loop body/condition ASTs instead
of rebuilding command structures on every iteration.

This attacked the nested-loop baseline directly and established the first rule
for later work: do not rebuild static command shape in a hot loop.

### 2. Command Bookkeeping

PR #11 avoided hot command bookkeeping overhead. The important class of fixes
was to skip shell state updates when the value was not observable by the current
command path.

This helped all four workloads and made function-heavy profiles clearer.

### 3. Lazy Function Call Arrays

PR #12 converted function call tracking arrays from eager shell-variable storage
to internal stacks:

- `FUNCNAME`
- `BASH_ARGC`
- `BASH_ARGV`
- `BASH_LINENO`
- `BASH_SOURCE`

Before this change, every function invocation formatted or updated Bash array
storage even when the script never read these variables. After the change, the
executor pushes/pops Rust `Vec` state during function calls and materializes the
array string only when expansion reads one of those variables.

This was a large win for `function-noop.sh` and `function-args-arith.sh`.

### 4. Avoid Hot Shell Environment Sync

PR #13 continued the same idea for regular assignments and command bookkeeping:

- ordinary non-exported shell assignments no longer call Windows
  `env::set_var`;
- assignment RHS tilde expansion skips HOME/USERPROFILE lookup when the value
  cannot need it;
- `_` is kept as shell-local state instead of being written to the process
  environment on every command.

This removed several Windows-specific costs from tight loops.

### 5. Skip Process-Substitution Work for Simple Commands

The profile still showed command materialization/cloning costs in simple
commands. PR #13 added a guard so simple commands that cannot contain process
substitution avoid the process-substitution materializer.

This kept the expensive path for real `<(...)` and `>(...)` cases, but let
ordinary arithmetic/function loop commands stay on the short path.

### 6. Lazy `PIPESTATUS`

`PIPESTATUS` was another eager array-string update. Profiles showed hot samples
under:

```text
Executor::set_pipestatus
store_indexed_array
format_indexed_array_values
```

The fix was to store pipeline statuses internally:

```rust
pipestatus: Vec<i32>
```

Then only materialize `PIPESTATUS` through dynamic array access when a script
actually reads it.

Important semantic detail: because `PIPESTATUS` moved out of `env_vars`, every
temporary environment snapshot needed a matching `pipestatus` snapshot. The
patch explicitly saved/restored it around:

- subshell execution;
- command substitution;
- function command substitution;
- process substitution;
- current-shell substitution paths.

Focused tests covered simple/pipeline statuses and `pipefail`.

### 7. Avoid Alias Clone on Simple Commands

After `PIPESTATUS`, `samply` still showed time under command expansion and alias
post-processing. The alias pass was cloning the expanded `CommandNode` even when
the alias table was empty.

The fix was small:

- consume the expanded command node instead of borrowing and cloning it;
- return immediately when `self.aliases.is_empty()`;
- only replace `words` when aliases actually need expansion.

This avoided a per-command clone in the no-alias hot path while preserving the
existing alias behavior.

### 8. Avoid Full Command Clone During Word Expansion

The next profile pointed at `expand_command_words`. It cloned the whole
`CommandNode`, including many parse-time metadata vectors and compound-command
fields that a simple expanded command no longer needed.

The optimized path builds a smaller execution-time command node:

- always preserve fields still needed by execution, assignments, redirects,
  heredocs, process substitution, background/connector flags, line info;
- preserve quote/process-substitution metadata only for paths that still inspect
  it later;
- preserve structured `conditional_command`, `arithmetic_command`, and
  `brace_group` because later dispatch uses those fields.

A first version dropped conditional metadata too aggressively and broke
`[[ ... ]]` quoted RHS semantics. The full `executor_tests` suite caught this in
the part 072 conditional tests. The fix was to preserve the conditional command
structure and quote metadata when needed.

## Validation Pattern

Each retained optimization had to pass this sequence:

```powershell
cargo fmt
cargo build --release --locked
cargo test --locked --lib
```

Focused executor tests were run around the touched behavior. Examples:

```powershell
cargo test --locked --test executor_tests test_pipestatus_tracks_simple_and_pipeline_statuses
cargo test --locked --test executor_tests test_pipefail_status_keeps_pipestatus_entries
cargo test --locked --test executor_tests test_underscore_tracks_last_command_argument
cargo test --locked --test executor_tests test_function_assignment_arithmetic_expansion_accepts_base_hash
cargo test --locked --test executor_tests test_conditional_quoted_regex_rhs_matches_literal_text
cargo test --locked --test executor_tests test_embedded_input_process_substitution_rewrites_external_argument
```

The full executor suite was also run:

```powershell
cargo test --locked --test executor_tests
```

On the local Windows/Git Bash environment, this consistently reported the same
three baseline failures:

- `test_exec_c_clears_external_command_environment`
- `test_command_v_without_p_uses_current_path_for_external_command`
- `test_command_without_p_uses_current_path_for_external_command`

Those were reproduced on baseline and treated as environment/PATH issues, not
new regressions.

## Final PR #13 Result

PR #13 merged at:

```text
427a85b387f99f2d5940cb12d4734d3c2765d1ce
```

The final benchmark recorded in the PR description used 15-run medians:

| Case | Baseline | Current | Delta |
| --- | ---: | ---: | ---: |
| `loop` | 315.38ms | 146.75ms | -53.5% |
| `arith` | 412.91ms | 163.76ms | -60.3% |
| `function-noop` | 427.38ms | 212.35ms | -50.3% |
| `function-args-arith` | 680.00ms | 259.22ms | -61.9% |

Same run versus Git Bash:

| Case | Current Rubash | Git Bash | Rubash / Bash |
| --- | ---: | ---: | ---: |
| `loop` | 146.75ms | 126.97ms | 1.16x |
| `arith` | 163.76ms | 141.16ms | 1.16x |
| `function-noop` | 212.35ms | 156.83ms | 1.35x |
| `function-args-arith` | 259.22ms | 216.44ms | 1.20x |

Rubash did not consistently beat Git Bash in this pass, but the gap narrowed
substantially, especially for arithmetic and function-with-args workloads.

## Rejected or Reverted Attempts

Not every plausible optimization was kept.

One attempted follow-up rewrote `expand_command_words` into a single-pass
expand-and-glob loop. Focused tests passed, but repeated benchmarks showed
function-heavy workloads getting worse or too noisy to trust. That patch was
reverted before committing.

The rule used there should remain the default: if a patch makes the code more
clever but does not produce a stable median improvement, drop it.

## Practical Lessons

- Use sampling first. Hand-written timers are useful only as a last resort; they
  add noise and are easy to forget to remove.
- On this Windows machine, WPR/xperf was installed but blocked by policy.
  `samply` was the practical fallback.
- Build with release optimizations when profiling. Debug builds point at the
  wrong costs.
- Add debug symbols and frame pointers when the profiler output is address
  heavy.
- Eager Bash compatibility state is expensive in tight loops. Prefer internal
  Rust state plus lazy materialization for dynamic shell variables.
- Be careful with quote metadata. `[[ ... ]]`, regex RHS, process substitution,
  and alias reparse paths often depend on metadata that ordinary simple
  commands do not need.
- Always compare against both `origin/master` and Git Bash. A patch can improve
  Rubash while still leaving the Bash gap unchanged.
- Keep dirty main worktrees out of the loop. Clean worktrees made it possible to
  merge PRs and inspect local changes without overwriting user work.

## Suggested Next Pass

The next likely targets are:

- remaining `expand_command_words` allocation and iterator overhead;
- `expand_word_mut` and parameter expansion fast paths for simple `$1`, `$2`,
  and literal words;
- function dispatch fixed costs around temporary assignment handling and local
  scope setup;
- shell option lookups that repeatedly hash strings in tight paths.

For the next pass, start with `samply` on `function-noop.sh` and
`function-args-arith.sh`, then only patch one hotspot at a time.
