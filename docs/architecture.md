# Rubash Engine Architecture

> Moved from the README on 2026-10-10, when the README was restructured to be
> evidence-first. Content is preserved from the README as of commit `7be6a799`
> (docs/readme: minimal style pass #495). `docs/source-layout.md` is archived
> and predates the current tree; this file carries the narrative.

## Subsystems

```
src/
├── lexer/           Tokenizer (quoting, escaping, heredocs, continuations)
├── parser/          Recursive-descent (simple cmds, pipelines, case, arith-for, [[ ]])
├── executor/        Command execution, builtins, expansion, glob, arrays, traps
├── builtins/        40+ builtin implementations (declare, read, printf, kill, ...)
└── lib.rs           Core types and error handling
```

- **Lexer**: Bash-style quoting, escaping, comments, variables, command substitution, arithmetic expansion, here-doc/here-string tokens, common redirects.
- **Parser**: Simple commands, pipelines, AND/OR lists, functions, brace/subshell groups, `if`, `for`, arithmetic `for`, `while`, `until`, `case`, `select`, `[[ ... ]]`, `coproc`, `time` prefixes.
- **Executor**: External commands, pipelines, redirects, temporary assignments, function calls, `source`/`.`, `eval`, shebangless script fallback, Windows/Git Bash path bridging.
- **Expansion**: Variables, positional parameters, indexed and associative arrays, command substitution, arithmetic expansion, brace expansion, tilde expansion, pathname globbing, `${parameter...}` operators, case/replacement transforms.
- **Builtins**: `alias`, `cd`, `declare`/`typeset`/`local`, `echo`, `eval`, `exec`, `export`/`readonly`, `getopts`, `hash`, `jobs`, `kill`, `let`, `mapfile`, `printf`, `pushd`/`popd`/`dirs`, `read`, `return`, `set`, `shopt`, `source`, `test`/`[`, `trap`, `type`, `ulimit`, `umask`, `unset`, `wait`, and more.

## Subshells without fork

POSIX `fork()` has no Win32 equivalent. Emulation layers (MSYS2, Cygwin) fake it at the syscall level — expensive, fragile, and the source of their best-known quirks. Rubash reproduces fork's *semantics* instead, at three layers:

1. **In-process subshells.** `( list )` and `$( )` never spawn a process. `ShellState::clone` produces the child's variables, aliases, functions, traps, and history — the memory side of a fork — and the copy is discarded when the subshell ends.
2. **A real-handle fd table with POSIX `dup` semantics.** Slots hold raw Windows `HANDLE`s, and `DuplicateHandle` duplicates share the same kernel file object — therefore the same file offset. That is exactly POSIX "dup shares the open file description", verified empirically in a POC before landing. `fork_table()` duplicates the whole table handle-by-handle, the way `fork` copies the fd table but not the file objects.
3. **Real processes only at true process boundaries.** External commands and pipeline members run via `CreateProcess` + `os_pipe`; background jobs and coprocs get their own processes, with job control on Job Objects and SIGCONT via `ResumeThread`.

The result: subshells and command substitutions pay zero process-creation cost, while everything a script can observe — exit codes, fd inheritance, shared offsets, signal dispositions — behaves like GNU Bash.

## Testing the engine

```bash
# Unit + integration tests
cargo test --lib

# bashdb compatibility
cargo test --test cli_tests bashdb_compat -- --nocapture

# Source expansion
cargo test --test cli_tests source_expands -- --nocapture
```

The engine also runs the [bashdb](https://github.com/Trepan-Debuggers/bashdb) core debugger loop (list, step, next, where, continue, quit) end-to-end. Fixture setup and smoke test: [`docs/bashdb-debugging-rubash.md`](bashdb-debugging-rubash.md).

## Working rules

- Fix by root cause subsystem, not by individual expected-output lines.
- Keep bashdb external and clean; temporary instrumentation is for diagnosis only.
- Every failing bashdb command is an opportunity to find and fix a Rubash compatibility gap.
- Compatibility baseline is GNU Bash 5.3.0 (owner-compiled at `/usr/local/bin/bash`).

Methodology rules for compatibility work: [`AGENTS.md`](../AGENTS.md) and [`docs/PROVENANCE.md`](PROVENANCE.md).
