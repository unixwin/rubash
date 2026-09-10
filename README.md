# Rubash

A GNU Bash-compatible shell implementation written in Rust.

[中文](README.zh-CN.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![Crates.io](https://img.shields.io/crates/v/rubash)](https://crates.io/crates/rubash)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## Overview

Rubash is an in-progress GNU Bash-compatible shell. It reimplements Bash lexical analysis, parsing, expansion, execution, and builtin behavior in Rust. The project is past the early skeleton stage; current work focuses on closing GNU Bash compatibility gaps, expanding upstream test coverage, and supporting Windows-native shell use cases such as Winuxsh.

**Current positioning**: Rubash is suitable for compatibility development, testing, research, and validating the Rubash execution engine/API. It is not yet declared production-ready as a login shell or critical script runtime.

## Current Status

The current source package version is `1.0.0`. Compatibility status is tracked
in [`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md), the single
authoritative source for Rubash ↔ GNU Bash standing. That document is updated
only after real reproduction against the GNU Bash test files vendored under
`third_party/bash/tests/`, with WSL GNU Bash 5.2.21 as the comparison
baseline. Dated analysis snapshots elsewhere under `docs/` are historical only
and must not be used to judge current parity. The `87/87` upstream `run-*`
runner result compares against the older `.right` expectation files and must
not be conflated with real-output parity.

Isolated GNU Bash semantics — arrays, associative arrays, arithmetic,
conditionals, namerefs, mapfile, POSIX command substitution, prefix brace
forms (`foo{a,b}`), and escaped-brace/escape-comma forms — match GNU Bash in
isolation. The remaining high-priority gaps are tracked per test file in the
status document (`posixexp2`, `cond`, `mapfile`, `comsub-posix`, and `braces`
sequence details).

Rubash's GNU Bash syntax and runtime support have progressed far enough to run complex Bash programs. A clean external `bashdb` checkout now completes the core debugger loop under `target/debug/rubash.exe`.

Verified bashdb core commands include:

- `list`: show the target script source.
- `step`: enter shell function bodies.
- `next`: advance to the next source line.
- `where`: print the call stack.
- `continue`: resume the debugged script.
- `quit`: exit the debugger.

This proves coverage for a substantial set of Bash semantics that bashdb depends on: `source`/`.`, `eval`, indexed and associative arrays, `DEBUG`/`RETURN`/`EXIT` traps, `BASH_SOURCE`, `BASH_COMMAND`, function stacks, `functrace`/`extdebug`, path expansion, redirects, `/dev/stdin`, `tty`, dynamic fds, parameter expansion, and arithmetic commands.

The boundary is important: **the bashdb core workflow is usable, but the complete bashdb command surface is not yet fully certified**. The goal is to make as much of bashdb as possible work under Rubash and use bashdb as a real Bash application stress test for finding more compatibility bugs.

## Feature Overview

- **Lexer**: Bash-style quoting, escaping, comments, variables, command substitution, arithmetic expansion, here-doc/here-string tokens, and common redirects.
- **Parser**: Simple commands, pipelines, AND/OR lists, functions, brace/subshell groups, `if`, `for`, arithmetic `for`, `while`, `until`, `case`, `select`, `[[ ... ]]`, `coproc`, and `time` prefixes.
- **Executor**: External commands, pipelines, redirects, temporary assignments, function calls, `source`/`.`, `eval`, shebangless script fallback, and Windows/Git Bash path bridging.
- **Expansion system**: Variables, positional parameters, indexed and associative arrays, command substitution, arithmetic expansion, brace expansion, tilde expansion, pathname globbing, common `${parameter...}` operators, and case/replacement transforms.
- **Array semantics**: Indexed and associative arrays, compound assignment, element assignment/append, negative indexes, slices, `${arr[@]}`/`${arr[*]}`, and common `declare`/`local`/`export`/`readonly` interactions.
- **Builtins**: Common Bash builtins are implemented or wired in, including `alias`/`unalias`, `builtin`, `cd`, `command`, `declare`/`typeset`/`local`, `echo`, `enable`, `eval`, `exec`, `exit`, `export`/`readonly`, `getopts`, `hash`, `help`, `jobs`, `kill`, `let`, `mapfile`/`readarray`, `printf`, `pushd`/`popd`/`dirs`, `pwd`, `read`, `return`, `set`, `shift`, `shopt`, `source`/`.`, `test`/`[`, `times`, `trap`, `type`, `ulimit`, `umask`, `unset`, and `wait`.
- **Known limitations**: Full job control, interactive readline/history, and process-group/terminal control remain host-sensitive. Bash parser and alias reread edge cases, full bashdb command coverage, and residual upstream compatibility cases are still under active compatibility work.

## Quick Start

### Build from Source

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

### Install Release

```bash
cargo install rubash
```

### Run a Script

```bash
target/debug/rubash path/to/script.sh
target/debug/rubash -c 'echo hello from rubash'
```

## Debugging Script Behavior with bashdb

bashdb is an external Bash script debugger. Rubash does not embed or vendor bashdb. The verified clean fixture is:

```bash
target/bashdb-clean/bashdb-generated
```

Core smoke test:

```bash
export TERM=xterm DARK_BG=0
printf 'list\nstep\nnext\nwhere\ncontinue\nwhere\nquit\n' | \
  target/debug/rubash.exe target/bashdb-clean/bashdb-generated --no-highlight target/bashdb-probe-target.sh
```

A passing run exits `0`, has empty stderr, shows target source for `list`, enters a function with `step`, shows the call stack with `where`, and continues through `42` / `done`.

The next goal is broad bashdb command coverage under Rubash. When another bashdb command fails, treat it first as a Rubash Bash-compatibility gap to root-cause rather than patching bashdb. See `docs/bashdb-debugging-rubash.md` for fixture setup, launcher/libdir terminology, and fresh-checkout usage.

## Testing

Focused tests commonly used for this area:

```bash
cargo test --test cli_tests bashdb_compat -- --nocapture
cargo test --test cli_tests source_expands -- --nocapture
cargo test --test cli_tests script_bash_source -- --nocapture
```

The full compatibility suite is still expanding. For compatibility work, prefer focused tests and bounded upstream GNU Bash slices instead of unbounded full-suite runs.

## Documentation

- `docs/COMPATIBILITY-STATUS.md`: authoritative Rubash ↔ GNU Bash compatibility status (single source of truth).
- `docs/bashdb-debugging-rubash.md`: bashdb fixture, launcher/libdir explanation, smoke test, and fresh-checkout usage.
- `docs/gnu-bash-compatibility-implementation-plan.md`: GNU Bash compatibility implementation plan.
- `docs/issue-suite-diff-analysis.md`: upstream test diff analysis.
- `docs/bash-compat-issues.md`: compatibility issue list.
- `docs/bash-source-map.md`: Bash source and semantic mapping.
- `docs/typed-expansion-migration-checkpoint.md`: typed expansion migration checkpoint.
- `docs/source-layout.md` and `docs/semantic-ownership.tsv`: source layout and GNU semantic ownership map.

## Development Principles

- Fix Rubash subsystems by Bash root cause, not by individual expected-output lines.
- Keep bashdb external and clean. Temporary instrumentation is acceptable only for diagnosis and must be reverted.
- bashdb is a high-level script behavior debugger, not a Rust source debugger. Use Rust tooling, logs, instrumentation, and focused tests for `src/**/*.rs` internals.
- Full bashdb usability is a development target. Every failing bashdb command is an opportunity to expose and fix a Rubash compatibility gap.

## License

Rubash is licensed under the MIT License. See `LICENSE`.

## Contributing

Issues, compatibility repros, focused regression tests, and implementation patches are welcome. Read `AGENTS.md` before compatibility work.

## Contact

- GitHub Issues: https://github.com/unixwin/rubash/issues
- Discussions: https://github.com/unixwin/rubash/discussions

## Acknowledgements

- GNU Bash team - original Bash implementation
- Trepan-Debuggers/bashdb - external Bash debugger and compatibility stress source
- Rust community - language and tooling

---

*Last updated: 2026-08-29*
