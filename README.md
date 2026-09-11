# Rubash

A GNU Bash-compatible shell implementation written in Rust.

[中文](README.zh-CN.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## What Is Rubash

Rubash is a from-scratch reimplementation of GNU Bash in Rust — lexer, parser, expansion engine, executor, builtins, and all. It targets byte-level compatibility with GNU Bash 5.3.0 and runs on Windows natively.

**Current status**: 32 out of 83 GNU Bash upstream test suites pass with zero difference. Total remaining diff across all 83 suites is 2072 lines, down from 3427 two days ago (−40%). Full details in [`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md).

## Compatibility at a Glance

```
GNU Bash 5.3.0 test suite — 83 files, true-baseline measurement

  PASS (0 diff):   32 suites  ████████████████░░░░░░░░░░░░░░░░  39%
  DIFF (1-50):     37 suites  █████████████████████████████████  45%
  DIFF (51-250):   14 suites  ████████████░░░░░░░░░░░░░░░░░░░░  17%
  ────────────────────────────────────────────────────────────────
  Total diff:      2072 lines (was 3427 on Sep 9 → −40% in 2 days)
```

### Fully passing suites (zero diff)

`appendop` `attr` `builtins` `casemod` `complete` `cprint` `dbg-support` `dbg-support2` `dstack2` `dynvar` `extglob2` `extglob3` `func` `getopts` `glob-bracket` `herestr` `ifs` `invert` `invocation` `mapfile` `nquote2` `nquote3` `nquote4` `nquote5` `posixexp2` `posixpat` `precedence` `printf` `rsh` `strip` `tilde` `tilde2` `trap`

### Major recent fixes (Sep 2026)

| Area | Before → After | What changed |
|------|----------------|-------------|
| **dbg-support** | 635 → 0 | AND-list dual fire, source-scope trap inheritance, `{` regression |
| **rsh** | 194 → 0 | `set +o restricted` silent lift, full restricted-shell enforcement |
| **invocation** | 14 → 0 | `BASH_ARGV0`, long options, `--pretty-print`, `-o`/`-O` prologs |
| **trap** | 3 → 0 | ERR line binding, SIGCHLD queue, background child trap isolation |
| **func** | 58 → 0 | POSIX funcname rules, AST printer, special-builtin precedence |
| **complete** | 115 → 0 | Multi-operand compspec registration |
| **history** | 190 → 127 | `history -d start-end` range deletion (GNU 5.3 feature) |
| **globstar** | 182 → 101 | Multiplicity fix, adjacent-`**` collapse, trailing-slash semantics |
| **array/assoc** | 444+358 → 246+242 | Compound assignment quote grouping, element-assignment boundaries |
| **signals** | BSD table → Linux table | USR1=10, CHLD=17, RTMIN=34, matching GNU 5.3.0 WSL contract |

### What Rubash can already run

- **bashdb** — core debugger loop (list, step, next, where, continue, quit) works under rubash
- **Complex Bash scripts** — arrays, associative arrays, arithmetic, conditionals, namerefs, command substitution, brace expansion, process substitution, coproc, `eval`, `trap`, `source`
- **GNU Bash test suite** — 83 upstream test files with automated diff measurement

## Quick Start

### Build from Source

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

### Run a Script

```bash
target/debug/rubash path/to/script.sh
target/debug/rubash -c 'echo hello from rubash'
```

### Run the Compatibility Suite

```bash
# Full 83-suite measurement (requires WSL with GNU Bash 5.3.0)
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh

# Single suite
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh array
```

## Architecture

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

## Testing

```bash
# Unit + integration tests
cargo test --lib

# bashdb compatibility
cargo test --test cli_tests bashdb_compat -- --nocapture

# Source expansion
cargo test --test cli_tests source_expands -- --nocapture
```

## Documentation

- [`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md) — **single source of truth** for Rubash ↔ GNU Bash compatibility status
- [`docs/builtins.md`](docs/builtins.md) — builtin inventory and dispatch model
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture setup and smoke test
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — how to run GNU Bash upstream tests

## Development Principles

- Fix by root cause subsystem, not by individual expected-output lines.
- Keep bashdb external and clean; temporary instrumentation is for diagnosis only.
- Every failing bashdb command is an opportunity to find and fix a Rubash compatibility gap.
- Compatibility baseline is GNU Bash 5.3.0 (owner-compiled at `/usr/local/bin/bash`).

## License

MIT — see [`LICENSE`](LICENSE).

## Contributing

Issues, compatibility reproductions, focused regression tests, and implementation patches welcome. Read [`AGENTS.md`](AGENTS.md) before compatibility work.

## Acknowledgements

- GNU Bash team — the original implementation being re-emplemented
- Trepan-Debuggers/bashdb — external debugger and compatibility stress test
- Rust community — language and tooling

---

*Last updated: 2026-09-11*
