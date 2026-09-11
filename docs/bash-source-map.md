> **ARCHIVED** — This document is historical. Current compatibility status is in
> [`COMPATIBILITY-STATUS.md`](COMPATIBILITY-STATUS.md). Last current: 2026-08-14.

# Bash Source Map

## Semantic Map v2

The canonical migration map is `docs/semantic-ownership.tsv`. It records
semantic ownership rather than pretending that one GNU C file has one Rust
translation. Its columns are:

| Column | Meaning |
|---|---|
| GNU source family | GNU C, `.def`, `.y`, or upstream test family |
| Semantic contract | Observable Bash behavior owned by the family |
| Rust owner | One or more Rust semantic owners, separated by `;` |
| Compile status | `active`, `unreferenced`, or `missing` |
| Implementation status | `real`, `partial`, `scaffold`, `bridge`, `deferred`, or `host-owned` |
| Suite evidence | Rust and upstream test families that exercise the owner |
| Next gate | Evidence required before the owner can be promoted |

The map is checked by `scripts/validate-semantic-map.sh`. A target file's
existence is not evidence of migration: `real` requires an active owner,
non-placeholder implementation, and named test evidence. One GNU family may
map to several owners, and several families may map to one semantic kernel.

The older file-by-file inventory below remains provenance data. New migration
work must update the v2 map first and may use the appendix for source lookup.

## Placeholder Audit

Run `scripts/audit-rust-placeholders.sh` to inspect Rust files that contain
only GNU provenance comments. The report distinguishes files declared by the
current module tree from unreferenced files, and separates host/deferred areas
(`input`, `readline`, `locale`, `sys`, completion, and history) from old
semantic-owner candidates.

`Code=0` is not an implementation status. An unreferenced file is removable
only after its provenance is represented in this map or the implementation
inventory. A duplicate-owner candidate is not automatically complete: the
replacement owner still needs behavior and suite evidence before it can be
marked `real`.

### Manually Confirmed Placeholder Exceptions

The following zero-code or low-code files are intentionally retained. A
`cloc --by-file` row is not enough evidence for deletion:

| Files | Classification | Reason |
|---|---|---|
| `src/executor/upstream_scripts/data.rs` | active structural module | Re-exports the upstream handler data modules used by the bridge. |
| `src/builtins/printf/identifier.rs` | real helper | Contains the active `valid_identifier` implementation used by printf. |
| `src/shell/mod.rs`, `src/shell/arrays/mod.rs`, `src/expand/mod.rs`, `src/expand/tilde/mod.rs`, `src/jobs/mod.rs` | active structural modules | Declare or re-export active semantic owners. |
| `src/complete/*`, `src/history/*`, `src/input/*` | deferred/host-owned | Interactive completion, history, readline, and termcap are outside the current noninteractive migration kernel. |
| `src/locale/*` | deferred/host-owned | GNU gettext and locale portability is host/environment work, not a Rubash semantic kernel. |
| `src/sys/*` | host-owned/deferred | GNU portability headers and POSIX helper shims are replaced selectively by Rust/std/Windows facilities. |
| `src/jobs/signals.rs`, `src/jobs/siglist.rs` | host-owned/deferred | Windows process/event delivery is owned by the host backend and the active builtin/trap owners; no standalone POSIX signal-table port is planned. |

These files must remain listed as deferred or host-owned until a replacement
owner and test gate exist. They must not be promoted to `real` merely because
the path exists.

This map keeps Rubash implementation work traceable to GNU Bash 5.3 sources
without forcing a file-for-file port. The `Status` column describes whether the
Rubash module should exist now or later.

## Upstream Inventory

The pinned GNU Bash submodule currently contains 1603 tracked files. The files
that most directly shape Rubash implementation are the C sources, headers,
builtin definitions, and parser grammar:

| Group | Count | Notes |
|---|---:|---|
| Total tracked files | 1603 | Full GNU Bash source tree, including docs, tests, translations, build support, and examples. |
| `.c` files | 301 | C implementation files across the root, `builtins/`, `lib/`, examples, and support tools. |
| `.h` files | 141 | C headers and generated/config headers. |
| `builtins/*.def` files | 43 | Bash builtin command definitions. |
| `.y` files | 2 | Parser grammars, including `parse.y`. |
| C/header/def/parser total | 487 | The main implementation-shaped inventory Rubash should track semantically. |
| `tests/` files | 738 | Upstream conformance and regression suite data. |
| `lib/` files | 316 | Readline, glob, tilde, sh portability helpers, malloc, intl, termcap. |
| `builtins/` files | 56 | Builtin definitions plus helper code. |
| `doc/` files | 37 | Manual/reference documentation. |

This document maps those files at subsystem granularity. The file-by-file owner
map lives in `docs/bash-implementation-inventory.md`; it assigns every
implementation-shaped GNU Bash file to a Rubash target module or an explicit
skip category. When a Rubash module is added or moved, update both maps as
needed.

| GNU Bash source | Rubash module | Status | Notes |
|---|---|---:|---|
| `parse.y`, `parser.h`, `y.tab.c`, `y.tab.h` | `src/parser/` | Now | Parser grammar reference only; do not mirror generated `y.tab.*`. |
| `command.h`, `make_cmd.c`, `copy_cmd.c`, `dispose_cmd.c`, `print_cmd.c` | `src/parser/nodes.rs`, `src/parser/parse_loop.rs` | Now | Rust AST and parser lifecycle are owned by active parser modules, not GNU-named allocation helpers. |
| `subst.c`, `subst.h` | `src/expand/parameter.rs`, `src/expand/command.rs` | Now | Parameter, command, arithmetic, quote removal, and word expansion logic. |
| `braces.c`, `bracecomp.c` | `src/expand/braces.rs` | Now | Brace expansion can be implemented independently and tested early. |
| `pathexp.c`, `lib/glob/glob.c`, `lib/glob/strmatch.c` | `src/executor/glob.rs`, `src/parser/pathname_pattern.rs` | Now | Pathname expansion and shell pattern matching use one active executor owner plus parser metadata. |
| `lib/tilde/tilde.c` | `src/expand/tilde/tilde.rs` | Now | Needed by `cd`, assignments, and word expansion. |
| `execute_cmd.c`, `execute_cmd.h`, `eval.c` | `src/executor/command_execute.rs`, `src/executor/compound_exec.rs`, `src/executor/pipeline_exec.rs` | Now | Main command execution is distributed across dispatch, compound execution, and pipeline owners. `src/executor/command.rs` is not an active module. |
| `redir.c`, `redir.h` | `src/executor/redirection.rs` | Now | File descriptor and redirect semantics. |
| `findcmd.c`, `hashcmd.c`, `hashlib.c` | `src/executor/path.rs`, `src/builtins/hash.rs` | Later | Command lookup and builtin hash behavior are separate owners. |
| `variables.c`, `variables.h` | `src/shell/variables.rs` | Now | Shell variables, exported environment, special parameters. |
| `flags.c`, `shell.c`, `shell.h` | `src/executor/shell_options.rs`, `src/shell/state.rs` | Now | Shell options and shared runtime state have separate active owners. |
| `builtins/*.def`, `builtins/common.c` | `src/builtins/` plus distributed executor owners | Now | Keep builtin-facing behavior in `src/builtins`; execution-heavy families such as `getopts`, `mapfile`, `read`, `let`, `break`, and `return` are owned by executor modules listed in the inventory. `common.c` is shared GNU support, not a standalone Rust module. |
| `test.c`, `builtins/test.def` | `src/builtins/test.rs` | Now | `test` and `[` behavior should share one implementation. |
| `alias.c`, `alias.h`, `builtins/alias.def` | `src/executor/alias_*.rs`, `src/builtins/alias.rs` | Later | Alias expansion and the builtin interface are separate active owners. |
| `array.c`, `array2.c`, `arrayfunc.c`, `assoc.c` | `src/shell/variables.rs`, `src/shell/arrays/`, `src/executor/arrays.rs` | Later | Keep typed storage and command execution ownership separate; do not create GNU-named shell placeholder files. |
| `jobs.c`, `nojobs.c`, `jobs.h` | `src/jobs/table.rs`, `src/executor/job_builtins.rs` | Later | Job state is a Windows-compatible semantic table; POSIX process-group helpers are not standalone Rust owners. |
| `trap.c`, `sig.c`, `siglist.c` | `src/builtins/trap.rs`, `src/executor/trap_exec.rs`; host-owned signal delivery | Later | Trap behavior and host signal delivery are separate contracts. |
| `input.c`, `bashline.c`, `lib/readline/*` | `src/input/` or external line editor | Later | Prefer crate-backed line editing before considering Bash readline parity. |
| `pcomplete.c`, `pcomplib.c`, `builtins/complete.def` | `src/complete/` | Later | Depends on readline/input and shell metadata. |
| `bashhist.c`, `lib/readline/history.c` | `src/history.rs` | Later | Interactive-only feature. |
| `locale.c`, `bashintl.h`, `po/`, `lib/intl/` | `src/locale.rs` | Defer | Not needed for early conformance. |
| `lib/sh/*` | `src/sys/` or standard library replacements | Selective | Most files are portability helpers; use Rust std/nix equivalents instead of porting. |
| `tests/*.tests`, `tests/*.right`, `tests/*.sub` | `scripts/run-bash-upstream-tests.sh` | Now | Keep upstream tests in the submodule and run the project harness for compatibility baselines. |

## Compatibility Target

The target is GNU Bash 5.3 observable behavior, including default Bash mode and
POSIX mode. Bash itself documents differences between default mode and POSIX
mode in `third_party/bash/POSIX`, and user-visible version differences in
`third_party/bash/COMPAT`.

Rubash progress should be measured by:

- Rust unit and integration tests for local implementation details.
- GNU Bash upstream `tests/run-*` progress.
- Focused differential tests against GNU Bash for newly implemented behavior.
