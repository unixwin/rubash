# Bash Source Map

This map keeps Rubash implementation work traceable to GNU Bash 5.3 sources
without forcing a file-for-file port. The `Status` column describes whether the
Rubash module should exist now or later.

| GNU Bash source | Rubash module | Status | Notes |
|---|---|---:|---|
| `parse.y`, `parser.h`, `y.tab.c`, `y.tab.h` | `src/parser/` | Now | Parser grammar reference only; do not mirror generated `y.tab.*`. |
| `command.h`, `make_cmd.c`, `copy_cmd.c`, `dispose_cmd.c`, `print_cmd.c` | `src/parser/ast.rs` | Now | Rust AST should model command semantics, not C allocation helpers. |
| `subst.c`, `subst.h` | `src/expand/parameter.rs`, `src/expand/command.rs` | Now | Parameter, command, arithmetic, quote removal, and word expansion logic. |
| `braces.c`, `bracecomp.c` | `src/expand/braces.rs` | Now | Brace expansion can be implemented independently and tested early. |
| `pathexp.c`, `lib/glob/glob.c`, `lib/glob/strmatch.c` | `src/expand/pathname.rs` | Now | Pathname expansion and shell pattern matching. |
| `lib/tilde/tilde.c` | `src/expand/tilde.rs` | Now | Needed by `cd`, assignments, and word expansion. |
| `execute_cmd.c`, `execute_cmd.h`, `eval.c` | `src/executor/command.rs` | Now | Main command execution flow. Keep high-level orchestration here. |
| `redir.c`, `redir.h` | `src/executor/redirection.rs` | Now | File descriptor and redirect semantics. |
| `findcmd.c`, `hashcmd.c`, `hashlib.c` | `src/executor/path.rs` or `src/shell/hash.rs` | Later | Command lookup and hashing after basic execution works. |
| `variables.c`, `variables.h` | `src/shell/variables.rs` | Now | Shell variables, exported environment, special parameters. |
| `flags.c`, `shell.c`, `shell.h` | `src/shell/options.rs`, `src/shell/status.rs` | Now | Shell options, invocation mode, exit status, runtime state. |
| `builtins/*.def`, `builtins/common.c` | `src/builtins/` | Now | Implement per builtin where useful, but group small builtins pragmatically. |
| `test.c`, `builtins/test.def` | `src/builtins/test.rs` | Now | `test` and `[` behavior should share one implementation. |
| `alias.c`, `alias.h`, `builtins/alias.def` | `src/shell/alias.rs` | Later | Needs parser/input integration before it is useful. |
| `array.c`, `array2.c`, `arrayfunc.c`, `assoc.c` | `src/shell/arrays.rs` | Later | Add after scalar variables and parameter expansion are stable. |
| `jobs.c`, `nojobs.c`, `jobs.h` | `src/jobs/` | Later | Requires process groups, terminal control, and signal semantics. |
| `trap.c`, `sig.c`, `siglist.c` | `src/jobs/signals.rs` or `src/shell/signals.rs` | Later | Implement with job control or script traps, not before. |
| `input.c`, `bashline.c`, `lib/readline/*` | `src/input/` or external line editor | Later | Prefer crate-backed line editing before considering Bash readline parity. |
| `pcomplete.c`, `pcomplib.c`, `builtins/complete.def` | `src/complete/` | Later | Depends on readline/input and shell metadata. |
| `bashhist.c`, `lib/readline/history.c` | `src/history.rs` | Later | Interactive-only feature. |
| `locale.c`, `bashintl.h`, `po/`, `lib/intl/` | `src/locale.rs` | Defer | Not needed for early conformance. |
| `lib/sh/*` | `src/sys/` or standard library replacements | Selective | Most files are portability helpers; use Rust std/nix equivalents instead of porting. |
| `tests/*.tests`, `tests/*.right`, `tests/*.sub` | `scripts/run-bash-upstream-tests.sh` | Now | Keep upstream tests in the submodule; add curated allowlists as features land. |

## Compatibility Target

The target is GNU Bash 5.3 observable behavior, including default Bash mode and
POSIX mode. Bash itself documents differences between default mode and POSIX
mode in `third_party/bash/POSIX`, and user-visible version differences in
`third_party/bash/COMPAT`.

Rubash progress should be measured by:

- Rust unit and integration tests for local implementation details.
- GNU Bash upstream `tests/run-*` progress.
- Focused differential tests against GNU Bash for newly implemented behavior.
