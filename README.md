# Rubash

An embeddable GNU Bash-compatible shell engine, written from scratch in Rust.

[中文](README.zh-CN.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## What Is Rubash

Rubash is a from-scratch reimplementation of GNU Bash semantics in Rust, packaged as an **embeddable, headless engine** — lexer, parser, expansion engine, executor, builtins, and all. It targets byte-level compatibility with GNU Bash 5.3.0 and runs on Windows natively.

Rubash itself is not a shell product. It ships with a reference CLI used by the compatibility harness and tooling, while interactive shells are built *on top of* the engine: niubash embeds Rubash for all bash semantics and owns line editing, prompt rendering, and completions itself.

**Measured, not claimed**: compatibility is verified against GNU Bash's own 83-suite upstream test corpus — 82 suites byte-identical today, every remaining diff line individually audited (ledger below).

**Why native matters**: shells billed as "bash on Windows" (Git Bash, MSYS2) ship a ported bash that rides on a POSIX emulation layer (`msys-2.0.dll`), with fork emulation and path translation that leak quirks into every script. Rubash has no such layer — one self-contained binary speaking Win32 directly.

**Paths are first-class, not converted**: the MSYS model *guesses* which arguments look like paths and rewrites them — which is why every AI agent and script has to set `MSYS_NO_PATHCONV=1` to stop `/flags` from becoming `C:/Program Files/Git/flags`. Rubash inverts the model: Windows paths are the native currency. POSIX-style and WSL-style paths are accepted as input and resolved to real Windows paths, so what a native Windows program receives is always a valid Win32 path — no conversion heuristics, no `MSYS_NO_PATHCONV`, no surprises at the process boundary.

**Platform status**: Windows is the current focus and the only platform with the full stack today. macOS and Linux adaptation is planned but has not started. The engine's semantic model (in-process subshells, fd-table semantics, process boundaries) is deliberately platform-neutral, so the same 83-suite ledger is designed to travel to other platforms.

## Identity and Compatibility (rubash#154)

The engine presents a **selectable platform identity** instead of outsourcing
it to whatever `uname.exe` happens to sit on PATH. `uname` and `arch` are
engine builtins (full option parsing, coreutils/MSYS2 output shapes), and
`OSTYPE`/`MACHTYPE`/`HOSTTYPE` follow the same persona.

- **Default persona: MSYS2-compatible** (disclosed everywhere — this is a
  compatibility mask, not a claim of being an MSYS2 port):
  - `uname -s` → `MSYS_NT-<ver>` — or `MINGW64_NT-` / `UCRT64_NT-` /
    `CLANG64_NT-` … when `MSYSTEM` is set, mapping the value the same way
    the MSYS2 runtime does;
  - `uname -m` / `arch` → `x86_64` / `aarch64` (build arch);
  - `uname -r` → the Windows version string (`10.0-19044`, same string that
    finishes `uname -s`);
  - `uname -o` → `Msys`; `uname -a` → `sysname nodename release version
    machine Msys` (the Git Bash shape);
  - `OSTYPE=msys`, `MACHTYPE=<arch>-pc-msys` (bound `set_if_not`-style, as
    GNU variables.c:723-725 does — an inherited value wins).
  Ecosystem scripts that branch on `case "$(uname -s)" in MINGW*|MSYS*|
  CYGWIN*)` or `$OSTYPE` ∈ {msys, cygwin} take their best-tested path.
- **`RUBASH_IDENTITY=native` switches to the honest-native persona**:
  `uname -s` → `Windows_NT` (the native `%OS%` value), `uname -o` →
  `Windows`, `OSTYPE=windows`, `MACHTYPE=<arch>-pc-windows`. Tests and
  native-first users select this; unset the variable (or set any other
  value) to return to the default.

**Disclosure surfaces** (the persona must never be silent):

- `rubash --help` prints an Identity section (current persona + how to switch);
- `rubash --identity` prints the persona and every effective value
  (`uname -s/-m/-r/-o`, `arch`, `OSTYPE`, `MACHTYPE`);
- this README section and the repo-root `SKILL.md` disclosure section
  (so AI agents consuming the repo also know the persona semantics).

`uname`/`arch` are *hidden* fast-path builtins (like `sleep`/`dirname`):
`type`/`enable`/`compgen -b` keep reporting them as external commands, and
they work even when PATH carries no `uname.exe` at all.

## Compatibility at a Glance

```
GNU Bash 5.3.0 test suite — 83 files, true-baseline measurement
(ledger: 2026-09-25 slice re-run on master 71c933eb — no upstream-script
stubs, niu-mounted /bin/sh fixture, per-suite TMPDIR, foreground
timeout; environment-bound diffs counted as zero after audit)

  PASS (0 diff):   82 suites  █████████████████████████████░  99%
  DIFF (residual):  1 suite   ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   1%
  ────────────────────────────────────────────────────────────────
  Residual:        nameref 1 (RO_PID visible in declare -r — coproc
                   reap-timing race that GNU itself exhibits)
  Env-zeroed:      glob extglob test type ifs-posix read coproc
                   nquote errors intl quotearray trap — NTFS
                   filename/attr bits, locale, host-tool output,
                   /dev/tty, path form, coproc reap timing
  Was 3427 on Sep 9 → −99%+ raw
```

### Fully passing suites (zero diff, environment diffs excluded)

`alias` `appendop` `arith` `arith-for` `array` `assoc` `attr` `braces` `builtins` `case` `casemod` `complete` `comsub-eof` `comsub-posix` `comsub` `comsub2` `cond` `coproc` `cprint` `dbg-support` `dbg-support2` `dstack` `dstack2` `dynvar` `errors` `exp` `exportfunc` `extglob` `extglob2` `extglob3` `func` `getopts` `glob-bracket` `glob` `globstar` `heredoc` `herestr` `histexp` `history` `ifs-posix` `ifs` `intl` `invert` `invocation` `iquote` `jobs` `lastpipe` `mapfile` `more-exp` `new-exp` `nquote` `nquote1` `nquote2` `nquote3` `nquote4` `nquote5` `parser` `posix2` `posixexp` `posixexp2` `posixpat` `posixpipe` `precedence` `printf` `procsub` `quote` `quotearray` `read` `redir` `rhs-exp` `rsh` `set-e` `set-x` `shopt` `strip` `test` `tilde` `tilde2` `trap` `type` `varenv` `vredir`

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

### Subshells without fork

POSIX `fork()` has no Win32 equivalent. Emulation layers (MSYS2, Cygwin) fake it at the syscall level — expensive, fragile, and the source of their best-known quirks. Rubash reproduces fork's *semantics* instead, at three layers:

1. **In-process subshells.** `( list )` and `$( )` never spawn a process. `ShellState::clone` produces the child's variables, aliases, functions, traps, and history — the memory side of a fork — and the copy is discarded when the subshell ends.
2. **A real-handle fd table with POSIX `dup` semantics.** Slots hold raw Windows `HANDLE`s, and `DuplicateHandle` duplicates share the same kernel file object — therefore the same file offset. That is exactly POSIX "dup shares the open file description", verified empirically in a POC before landing. `fork_table()` duplicates the whole table handle-by-handle, the way `fork` copies the fd table but not the file objects.
3. **Real processes only at true process boundaries.** External commands and pipeline members run via `CreateProcess` + `os_pipe`; background jobs and coprocs get their own processes, with job control on Job Objects and SIGCONT via `ResumeThread`.

The result: subshells and command substitutions pay zero process-creation cost, while everything a script can observe — exit codes, fd inheritance, shared offsets, signal dispositions — behaves like GNU Bash.

## Quick Start

### Build from Source

> Full functionality requires Windows today.

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
# Full 83-suite measurement (requires WSL + GNU Bash 5.3.0)
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh

# Single suite
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh array
```

## Testing

```bash
# Unit + integration tests
cargo test --lib

# bashdb compatibility
cargo test --test cli_tests bashdb_compat -- --nocapture

# Source expansion
cargo test --test cli_tests source_expands -- --nocapture
```

The engine also runs the [bashdb](https://github.com/Trepan-Debuggers/bashdb) core debugger loop (list, step, next, where, continue, quit) end-to-end.

## Documentation

- [`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md) — **single source of truth** for Rubash ↔ GNU Bash compatibility status
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — provenance statement: what Rubash is relative to GNU Bash, and contributor methodology rules
- [`docs/builtins.md`](docs/builtins.md) — builtin inventory and dispatch model
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture setup and smoke test
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — how to run GNU Bash upstream tests

## Development Principles

- Fix by root cause subsystem, not by individual expected-output lines.
- Keep bashdb external and clean; temporary instrumentation is for diagnosis only.
- Every failing bashdb command is an opportunity to find and fix a Rubash compatibility gap.
- Compatibility baseline is GNU Bash 5.3.0 (owner-compiled at `/usr/local/bin/bash`).

## Provenance

Rubash is a **from-scratch rewrite of GNU Bash semantics in Rust** — not a port or translation of the GNU Bash C code. No GNU Bash source is compiled into, linked with, or copied into Rubash's own code. Compatibility is defined against the *observable behavior* of GNU Bash 5.3.0 and verified by black-box differential testing. The vendored GNU Bash source lives in the separate `third_party/bash` submodule under its original GPL-3.0-or-later license and serves only as a semantic reference and test oracle. Full statement and contributor rules in [`docs/PROVENANCE.md`](docs/PROVENANCE.md).

## License

MIT — see [`LICENSE`](LICENSE).

## Contributing

Issues, compatibility reproductions, focused regression tests, and implementation patches welcome. Read [`AGENTS.md`](AGENTS.md) before compatibility work.

## Acknowledgements

- GNU Bash team — the reference implementation whose observable behavior defines our compatibility target
- Trepan-Debuggers/bashdb — external debugger and compatibility stress test
- Rust community — language and tooling

---

*Last updated: 2026-09-23*
