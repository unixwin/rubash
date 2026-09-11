> **ARCHIVED** — This document is historical. The actual source layout has
> diverged. See `src/` for current structure. Last current: 2026-08-28.

# Rubash Source Layout

Rubash targets GNU Bash 5.3 observable behavior, but it should not mirror the
GNU Bash C source tree file-for-file. The Bash source tree mixes long-lived C
subsystems, generated parser files, autotools support, portability shims, and
interactive shell infrastructure. Rust modules should be organized around
semantic ownership, state boundaries, and testable behavior.

Use `docs/bash-source-map.md` to keep every Rubash subsystem traceable to the
corresponding GNU Bash source files and upstream test groups. Use
`docs/bash-implementation-inventory.md` for the file-by-file ownership map.
For current implementation sequencing, agent workflow, and the rubash /
winuxcmd / winuxsh architecture boundary, use
[`docs/gnu-bash-compatibility-implementation-plan.md`](gnu-bash-compatibility-implementation-plan.md).

## Target Layout

```text
src/
  main.rs
  lib.rs

  lexer/
    mod.rs

  parser/
    mod.rs
    ast.rs

  expand/
    mod.rs
    braces.rs
    bracecomp.rs
    tilde.rs
    parameter.rs
    command.rs
    arithmetic.rs
    word.rs
    pathname.rs
    glob/

  shell/
    mod.rs
    state.rs
    alias.rs
    arrays/
    options.rs
    runtime.rs
    status.rs
    variables.rs

  executor/
    mod.rs
    fd_table.rs
    redirection.rs
    pipeline.rs
    path.rs

  builtins/
    mod.rs
    echo.rs
    cd.rs
    pwd.rs
    export.rs
    unset.rs
    test.rs
    # Builtin implementations live here only when they own behavior.

  jobs/
    mod.rs
    table.rs
    # Job state is owned by table.rs; trap execution is in executor/trap_exec.rs.

  input/
    mod.rs
    bashline.rs
    input.rs
    readline/

  complete/
    mod.rs

  history/
    mod.rs

  locale/
    mod.rs

  sys/
    mod.rs
```

The target layout is intentionally more granular than a 25-file shell. The
pinned GNU Bash tree has 487 implementation-shaped files (`.c`, `.h`, `.y`, and
`builtins/*.def`). Most of those should have an explicit Rubash ownership target
even when they are not ported yet. See `docs/bash-implementation-inventory.md`.

Current scaffold status:

| Item | Count |
|---|---:|
| GNU Bash implementation-shaped files | 487 |
| Rubash Rust owner targets in inventory | 308 |
| Existing inventory owner target files | 308 |
| Total `src/**/*.rs` files, including `mod.rs` and entrypoints | 325 |
| Explicit skip categories | 3 |

The current semantic kernels are `executor/fd_table.rs`, `shell/state.rs`,
`shell/variables.rs`, and `jobs/table.rs`. They are owners with focused APIs;
their presence does not mean every caller has migrated. See
`docs/semantic-ownership.tsv` for the promotion gates.

## Create Now

- `src/parser/nodes.rs`: command AST, control-flow nodes, and shell syntax data
  structures.
- `src/expand/`: word expansion, parameter expansion, command substitution,
  tilde expansion, brace expansion, quote removal, glob/pathname expansion.
- `src/shell/`: shared runtime state such as variables, exported environment,
  shell options, and last status.
- `src/executor/redirection.rs`: file descriptor and redirect semantics.
- `src/executor/pipeline.rs`: real pipeline execution and process connection.
- `src/builtins/`: move builtins out of `executor` as behavior becomes real.

These modules correspond to features already claimed or partially represented in
the current code, so creating them reduces drift without introducing speculative
architecture.

## Defer

- `src/jobs/`: process groups, foreground/background jobs, terminal control,
  and signals. This depends on real process execution and pipelines.
- `src/input/` or `src/readline/`: interactive line editing and history. Prefer
  a Rust line-editor crate before attempting Bash readline parity.
- `src/complete/`: programmable completion depends on input/readline and shell
  metadata.
- `src/history.rs`, `src/locale.rs`, `src/mail.rs`: useful later, but not on the
  shortest path to upstream test progress.
- `src/shell/arrays.rs`: add after scalar variables and parameter expansion are
  stable.

## Policy

- Do not port GNU Bash C files line-by-line, but do assign each
  implementation-shaped upstream file to a Rubash owner.
- Do not mirror generated files such as `y.tab.c` and `y.tab.h`.
- Treat filename collisions by module ownership, not basename. For example,
  `src/builtins/kill.rs` owns the Bash `kill` builtin, while
  `src/input/readline/kill.rs` owns readline editing behavior.
- Every new semantic module should update `docs/bash-source-map.md` and, when
  needed, `docs/bash-implementation-inventory.md`.
- Every compatibility PR should name the upstream `tests/run-*` group it moves.
- Small builtins may share modules when that keeps the Rust implementation
  clearer; complex builtins should get their own files.
