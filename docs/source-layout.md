# Rubash Source Layout

Rubash targets GNU Bash 5.3 observable behavior, but it should not mirror the
GNU Bash C source tree file-for-file. The Bash source tree mixes long-lived C
subsystems, generated parser files, autotools support, portability shims, and
interactive shell infrastructure. Rust modules should be organized around
semantic ownership, state boundaries, and testable behavior.

Use `docs/bash-source-map.md` to keep every Rubash subsystem traceable to the
corresponding GNU Bash source files and upstream test groups. Use
`docs/bash-implementation-inventory.md` for the file-by-file ownership map.

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
    alias.rs
    arrays/
    options.rs
    runtime.rs
    status.rs
    variables.rs

  executor/
    mod.rs
    command.rs
    eval.rs
    hash.rs
    hashlib.rs
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
    common.rs
    getopt.rs

  jobs/
    mod.rs
    jobs.rs
    signals.rs
    trap.rs

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

## Create Now

- `src/parser/ast.rs`: command AST, control-flow nodes, and shell syntax data
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
- Every new semantic module should update `docs/bash-source-map.md` and, when
  needed, `docs/bash-implementation-inventory.md`.
- Every compatibility PR should name the upstream `tests/run-*` group it moves.
- Small builtins may share modules when that keeps the Rust implementation
  clearer; complex builtins should get their own files.
