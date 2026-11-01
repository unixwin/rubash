<p align="center">
  <img src="assets/rubash-lockup.svg" alt="rubash — embeddable bash-compatible engine" width="620" />
</p>

# Rubash

**A GNU Bash 5.3-compatible shell engine, written from scratch in Rust.** Embeddable as a library, native on Windows with **no POSIX emulation layer** — the engine inside [niubash](https://github.com/unixwin/niubash), and a from-scratch alternative to Git Bash / MSYS2's ported bash.

[中文](README.zh-CN.md) · [unixwin.dev](https://unixwin.dev)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/unixwin/rubash)](https://github.com/unixwin/rubash/releases)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Net code size](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/unixwin/rubash/master/docs/code-size-badge.json)](docs/code-size.md)

Rubash is a from-scratch reimplementation of GNU Bash semantics in Rust — lexer, parser, expansion engine, executor, builtins — packaged as an **embeddable, headless engine**. It targets byte-level compatibility with GNU Bash 5.3.x (the vendored upstream tree is at the Bash-5.3 patch 20 level, #957) and runs natively on Windows, Linux, and macOS.

Rubash itself is not a shell product. It ships a reference CLI used by the compatibility harness and tooling; interactive shells are built *on top of* the engine: [niubash](https://github.com/unixwin/niubash) embeds Rubash for all bash semantics and owns line editing, prompt rendering, and completions itself.

## Quick Start

### Get it

Prebuilt tarballs for Linux (x86_64) and macOS (aarch64) are attached to each [release](https://github.com/unixwin/rubash/releases) (latest: v1.4.3, as of 2026-10-30). On Windows, the engine ships inside [niubash](https://github.com/unixwin/niubash) — that is the binary to install; this repo ships no Windows binary of its own.

### Build from source

> Cross-platform: Windows is the full-stack primary target; Linux builds and runs natively with CI test lanes; macOS runs the library test suite in CI (see "Platform status" under [Why Rubash](#why-rubash)).

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

### Run a script

```bash
target/debug/rubash path/to/script.sh
target/debug/rubash -c 'echo hello from rubash'
```

### Run the compatibility suites

```sh
# GNU upstream corpus replay (the CI lane runner, Linux):
scripts/run-bash-upstream-tests.sh        # writes target/bash-upstream-tests/summary.md

# Full 83-suite true-baseline measurement (the harness behind the ledger;
# from Windows, per scripts/true-baseline.sh's own header):
MSYS_NO_PATHCONV=1 wsl bash /path/to/rubash/scripts/true-baseline.sh          # all 83 suites
MSYS_NO_PATHCONV=1 wsl bash /path/to/rubash/scripts/true-baseline.sh array    # single suite
```

## Why Rubash

**Measured, not claimed.** Compatibility here is three mechanical rulers, not an adjective:

- **Our own differential suite** — 246 original cases byte-compared against a locally built GNU Bash 5.3.20 (stdout, stderr, exit status): **244/246 identical**, up from 222/246 at the first audit (2026-10-22). The 2 residuals are documented platform differences.
- **GNU's own corpus** — the 83-suite upstream test corpus as the consistency yardstick; the full ledger is below.
- **The real ecosystem** — nvm, bats-core, bash-it, oh-my-bash, bash-completion, modernish, run for real. Every divergence becomes a public issue and a pinned regression test: as of 2026-10-30, rubash has 1,440 issues filed and 1,408 closed.

**Why native matters.** Git for Windows describes its own shell, verbatim, as "a BASH emulation used to run Git from the command line" — an emulation built to run Git. Its bash is a port riding on a POSIX emulation layer (`msys-2.0.dll`), and the emulation leaks: fork emulation, argument rewriting, device quirks in every script. Rubash has no such layer — one self-contained binary speaking Win32 directly.

**Paths are first-class, one dialect per child.** The MSYS model *guesses* which arguments look like paths and rewrites them; MSYS2's own documentation concedes the conversion "isn't perfect" and rewrites arguments that are not paths at all — which is why every AI agent and script ends up setting `MSYS_NO_PATHCONV=1` to stop `/flags` from becoming `C:/Program Files/Git/flags`. Rubash inverts the model with a per-child dialect contract (the same line GNU draws: `shell_execve` hands `execve` the raw word bytes and never rewrites argv). A child that owns a POSIX layer — anything resolved out of a WinuxCmd installation, or through the `winuxcmd` dispatcher — keeps the shell's spellings and resolves `/tmp`, `/dev/*` and the shell tree itself, with one documented exception: drive-form operands (`/d/...`) are translated by the engine funnel for every child class ([#592](https://github.com/unixwin/rubash/issues/592)). A native Windows program instead receives real Win32 spellings for classified path-shaped operands — virtual roots, `/mnt/X`, and drive forms with components such as `/d/repo` — translated whether or not the target exists yet, while the documented verbatim classes cross exactly as typed: the bare `/`, drive-root forms (`/d`, `/d/`), attached values (`--opt=/d/x`), and grammar-data operands ([#748](https://github.com/unixwin/niubash/issues/748), [#779](https://github.com/unixwin/niubash/issues/779)). The contract of record is niubash's `docs/src/windows-path-contract.md`. The translation surface is deliberately narrow: quoted arguments are never rewritten, and the one device translation is the exact token `/dev/null` → `nul` ([#1506](https://github.com/unixwin/rubash/issues/1506)). Every overreach the suites ever found ([#31](https://github.com/unixwin/rubash/issues/31), [#59](https://github.com/unixwin/rubash/issues/59), [#60](https://github.com/unixwin/rubash/issues/60)) became a public issue and a regression test. For emergency rollback, the pre-Option-B translation behavior is available by exporting `__RUBASH_ARGV_DIALECT=legacy`.

**Bash 5.3, not 5.2.** Git for Windows bundles a Bash 5.2-line shell. Rubash targets 5.3 semantics — including syntax that structurally does not exist in 5.2, such as no-fork command substitution `${ cmd; }` ([#195](https://github.com/unixwin/rubash/issues/195)).

**Subshells without fork.** `( list )` and `$( )` run **in-process** — no process creation, no fork emulation — while everything a script can observe (exit codes, fd inheritance, shared file offsets, signal dispositions) behaves like GNU Bash. The three-layer design is detailed under [Architecture](#architecture).

**Embeddable by design.** Rubash is a Rust library with a reference CLI, not just an executable: niubash links it in-process for all shell semantics today, and the 1.5.0 line (see [Roadmap](#roadmap)) formalizes that surface into a stable, versioned engine API.

**Platform status — cross-platform, Windows-first.** Windows is the primary target and the platform with the full stack: one self-contained binary solving the classic Windows bash pain points (no POSIX emulation layer, no path-conversion heuristics, no `MSYS_NO_PATHCONV`, native Win32 paths as the currency, MSYS2-compatible identity for the bash ecosystem). Linux builds and runs natively (`x86_64-unknown-linux-gnu`) with real getrlimit/chmod/faccessat/uname(2) semantics and signal delivery — CI runs the engine's test lanes on Ubuntu (library tests, compatibility `cli_tests`, marker goldens) and replays the GNU upstream corpus on every push (the `Bash upstream test progress` job, `scripts/run-bash-upstream-tests.sh`); an earlier manual pass of the same corpus on the Linux binary scored 17/24 suites byte-identical (2026-09-27, ledger archived in #477/#581), and the remaining gap is tracked in the issue tracker. macOS runs the library test suite in CI (`macos-latest`) alongside the cross-target check. The engine's semantic model (in-process subshells, fd-table semantics, process boundaries) is deliberately platform-neutral, so the same ledger travels.

## Rubash vs Git Bash / MSYS2

| | **Rubash** | **Git Bash / MSYS2** |
| --- | --- | --- |
| Implementation | Bash semantics reimplemented from scratch in Rust, speaking Win32 directly | GNU Bash ported onto the `msys-2.0.dll` POSIX emulation layer |
| Bash language level | **GNU Bash 5.3** semantics — including 5.3-only syntax such as no-fork command substitution `${ cmd; }` ([#195](https://github.com/unixwin/rubash/issues/195)) | GNU Bash **5.2** (the version Git for Windows bundles) |
| Argument path handling | Deterministic per-child dialect contract — classified spellings translated per child class, documented verbatim classes cross unchanged; quoted arguments never rewritten; no guessing, no kill-switch variable | Heuristic rewriting of path-shaped arguments — its own docs concede it "isn't perfect"; `MSYS_NO_PATHCONV` / `MSYS2_ARG_CONV_EXCL` are the documented opt-outs |
| Subshells & command substitution | In-process — zero process creation | `fork()` emulation in the runtime |
| Console device redirects | Behave as in GNU Bash — the engine splits `CON` into `CONIN$`/`CONOUT$` ([#1312](https://github.com/unixwin/rubash/issues/1312), [#1314](https://github.com/unixwin/rubash/issues/1314)) | Fail outright in the MSYS layer with `Invalid argument` (project-measured) |
| Device paths to native programs | `/dev/null` in a native program's arguments is translated to `nul`, uniformly with the rest of the argv ([#1506](https://github.com/unixwin/rubash/issues/1506)) | Translated inside the emulation layer |
| Embeddability | A Rust library; hosts link the engine in-process (niubash is the reference host) | A standalone executable; no embeddable engine |

**Cygwin**, the third incumbent, defines itself by the same architecture: its site describes a DLL "which provides substantial POSIX API functionality" and states plainly that applications "must rebuild your application *from source*" to run in its world. A parallel universe, rebuilt from source, on the other side of a translation layer — rubash is a native build instead.

## The same ruler, pointed at the other claimants

The shells that call themselves bash replacements deserve the same measurement. Of the Rust-era peers, hush is a Lua-based *language* that never claimed to be a bash replacement, and Oils/OSH bills itself as an "upgrade path from bash", not a drop-in — neither claims the thing we measure. **brush** does: "Run it as your everyday shell", and its compatibility page rates itself "Production-ready for most use cases" while conceding ~125 known failures as "mostly edge cases". Measured on our 306-case differential corpus (byte-compared against GNU Bash 5.3.20), brush's shipped v0.4.0 scores **181/306**; a source build of its latest master commit re-measures at **183/306** — the same failure profile. Rubash on the same corpus: **304/306**. The *shape* of the gap matters more than the number (minimal repros in the campaign evidence files):

- Its alias timing model is the reverse of GNU's: aliases expand in non-interactive shells without `expand_aliases`, take effect on the line that defines them, and expand at call time inside function bodies — 6 substantive divergences across 20 dedicated probes. Alias expansion is one of the simplest mechanisms in GNU semantics; a reversed timing model silently rewrites real `.bashrc` shapes.
- `printf x y` (a conversion-free format string) loops forever and hangs the script in the release (fixed on master).
- Job control in script mode is a façade: `wait $!` reports "not yet implemented"; 27 of 29 job cases diverge.
- Signal traps never fire; subshell EXIT traps are lost; globstar's `**` does not recurse — while its compatibility page lists globstar under "Fully Supported".
- Namerefs are half-built: `unset` of a nameref reports not-implemented, and a real ecosystem load (oh-my-bash) trips over exactly that.
- Exit codes and diagnostics are home-grown end to end (rc 99, ANSI color codes inside pipes) — programs and agents that decide from exit status and stderr get silently misled.
- Its headline platform is missing: Windows support is self-described "Preview", and the v0.4.0 release ships no Windows binary at all.

Lightweight is no excuse for missing limbs. Rubash's 292,309 net lines (tests excluded — the docs/code-size.md caliber; GNU Bash's tree nets 162,815) — 345,093 together with niubash's 52,784 — are the finished price of two giant jobs: complete bash semantics re-implemented path by path without fork, and cross-platform itself. An implementation claiming the same thing at roughly a fifth of the engine mass, which then fails these foundation mechanisms, can be neither a bash replacement nor a reliable login shell.

## Compatibility at a Glance

```
GNU Bash 5.3.x test suite — 83 files, true-baseline measurement
(ledger: 2026-10-12 full re-run on master 764b9dde, host WINUXSH_ROOT
pin scrubbed — no upstream-script stubs, niu-mounted /bin/sh fixture,
per-suite TMPDIR, foreground timeout; environment-bound diffs counted
as zero after audit; earlier slice: 2026-09-25 on 71c933eb)

  PASS:            83/83 suites  ██████████████████████████████  100%
  ────────────────────────────────────────────────────────────────
  Raw:             62/83 suites byte-identical stdout on Windows —
                   literal 0-diff runs, scripts/true-baseline.sh
  Env-classified:  +20 suites differ only on environment-bound lines
                   (host-tool output, NTFS filename/attr bits, locale,
                   /dev/tty, path form, signal table, coproc reap
                   timing) — per-line attribution in the ledger below
  Clean:           82/83 suites with no rubash-caused diff after the
                   per-line environment audit
  nameref:         1 line (`declare -r RO_PID` visible after coproc) —
                   proven GNU 5.3.0's own coproc reap-timing race: GNU
                   vs itself flips 59/60↔1/60 on the harness filesystem,
                   60/60 deterministic on ext4 where its output is
                   byte-identical to rubash (stdout and stderr) —
                   docs/nameref-suite-residual-gnu-race-20261012.md
  Was 3427 on Sep 9 → −99%+ raw
```

The 2026-10-12 full re-run predates the v1.4.2 tag.

### Fully passing suites (zero diff, environment diffs excluded)

The clean set is derived from the ledger's per-suite classification, not hand-maintained: a suite lands here when every differing line is environment-attributed (per AGENTS.md, split rubash-caused vs environment-caused with the covering issue). The raw count above (62) plus the environment-classified set (20) plus nameref's proven-GNU race account for all 83.

`alias` `appendop` `arith` `arith-for` `array` `assoc` `attr` `braces` `builtins` `case` `casemod` `complete` `comsub-eof` `comsub-posix` `comsub` `comsub2` `cond` `coproc` `cprint` `dbg-support` `dbg-support2` `dstack` `dstack2` `dynvar` `errors` `exp` `exportfunc` `extglob` `extglob2` `extglob3` `func` `getopts` `glob-bracket` `glob` `globstar` `heredoc` `herestr` `histexp` `history` `ifs-posix` `ifs` `intl` `invert` `invocation` `iquote` `jobs` `lastpipe` `mapfile` `more-exp` `new-exp` `nquote` `nquote1` `nquote2` `nquote3` `nquote4` `nquote5` `parser` `posix2` `posixexp` `posixexp2` `posixpat` `posixpipe` `precedence` `printf` `procsub` `quote` `quotearray` `read` `redir` `rhs-exp` `rsh` `set-e` `set-x` `shopt` `strip` `test` `tilde` `tilde2` `trap` `type` `varenv` `vredir`

(nameref — the 83rd suite — is accounted one block above: its single residual line is GNU Bash's own coproc reap race, proven byte-for-byte in `docs/nameref-suite-residual-gnu-race-20261012.md`.)

### Beyond the GNU suite — mixed evaluation suites and the real-world ecosystem

The same true-baseline method (script files, byte-level diff vs GNU 5.3.x) applied to dedicated evaluation suites and real-world corpora (full inventory + verdicts archived in issue #477):

| Surface | Result |
| --- | --- |
| **modernish** capability suite (cross-shell quirk probes) | **166/166 rc-vector identical** |
| **mvdan/sh parser corpus** | 559 snippets, parse-accept/exit parity after the operator-class fixes |
| **ble.sh** (29,601-line line-editor framework) | `bash -n` **rc=0 parity**; source mode terminates |
| **bash-completion** | 452/452 matrix; `bash -n` canary byte-identical |
| **git-completion** | 140/140 |
| **oh-my-bash** | 22 libs load, **252/252 functions** environment parity, PS1 byte-identical (5 themes) |
| **nvm.sh** (v0.40.x loader) | loads byte-clean incl. empty-`NVM_DIR` auto-use on Windows native (`$PWD` POSIX persona, #537); own test slice engine-caused failures fixed |
| **FFmpeg / OpenSSH / PHP / ltmain / cmake configure scripts** | `--help`/error-path output **byte-identical** |
| **git's own test framework** (t/test-lib.sh) | runs end-to-end under the engine |
| **GNU 83-suite on Linux native binary** | 17/24 byte-identical (2026-09-27 wt4/linuxrun lane, ledger archived in #477/#581); the corpus is now replayed in CI on every push (`Bash upstream test progress`, Ubuntu) |

Everything above is pinned as regression fixtures under `tests/` (906 integration test files as of 2026-10-30, incl. `tests/regression.rs` — GNU-golden matrices, suite-slice snapshots, and perf canaries) running in CI on every push — plus a C-source line-level audit (`docs/SOURCE-AUDIT.md`, 336 behaviors inventoried) that keeps finding and closing gaps before suites hit them.

## Performance

Measured, not claimed — under one protocol, published whether it flatters us or not. The reference asset is the 2026-10-25 baseline (PERF-BASELINE-2026-10-25 in the public [shellbench](https://github.com/unixwin/shellbench) repo): release builds, Linux, against a self-built GNU Bash 5.3.20; the in-repo ledger is [`docs/performance.md`](docs/performance.md) and perf canaries ride the CI regression suite.

**The honest headline.** On interpreter-bound micro-work, current release builds pay a roughly uniform **~3× floor tax** against GNU Bash 5.3.20 — arithmetic, function calls, assignment, and read loops land at 2.8–3.2× the per-op cost. It is one mechanism (dispatch cost), not hundreds of separate ones, and beating GNU across the full table is the project's stated performance goal: the hard gate for the flagship release after 1.5.0, tracked against this published baseline. The worst single shape in this baseline, `$((a[i]))` array-element arithmetic reads at ~11×, has since closed to 2.99× (2026-10-28 round, same protocol) — back at floor-tax level.

**Where the process model wins.** Workloads whose cost is process creation run the other way, because pipeline stages and command substitution execute in-process: the pipe benchmark costs **0.05×** GNU's time and the command-substitution loop **0.11×**; across the 409-case shellbench breadth table, the substitution class median is 0.538×. Breadth today, of 408 comparable cases: 53 ahead, 17 close, 338 behind — the behind set is the floor tax above, stated plainly.

**Startup — the one quotable set.** `-c 'true'`, p50 of 50 interleaved samples, release builds, Linux: **rubash 3.43 ms, GNU Bash 5.3.20 2.59 ms**. Windows startup figures are not quoted anywhere until they are re-measured on real Windows hardware under the same protocol.

## Roadmap

Both items below were designed in the open — scoped, anchored to the code, and finalized in design docs — and land in **1.5.0**.

- **Embeddable engine API** ([#595](https://github.com/unixwin/rubash/issues/595)): a stable, semver'd Session-level public contract — input injection, output streams, lifecycle, event callbacks, rc policy — with niubash fully migrated onto it as the reference host. Finalized design: [`docs/design/1.5.0-session-api.md`](docs/design/1.5.0-session-api.md); RFC draft v1: [`docs/design/1.5.0-workspace-and-engine-api.md`](docs/design/1.5.0-workspace-and-engine-api.md).
- **Native script debugger** ([#497](https://github.com/unixwin/rubash/issues/497)): `rubash --debug` (an interactive, gdb-grammar front end intended to replace the bashdb workflow) and `rubash --dap` (the same debug session over the Debug Adapter Protocol, for editors) as two views over one engine-owned event stream — the API's second real consumer. Finalized design: [`docs/design/debugger-full-design.md`](docs/design/debugger-full-design.md); M0 design: [`docs/design/debugger-m0.md`](docs/design/debugger-m0.md).

## Milestones

<p align="center">
  <img src="assets/milestone-timeline.svg" alt="The Climb — rubash milestone timeline: from a 3,427-line GNU-suite diff on 2026-09-09 to 83/83 upstream suites passing on 2026-10-12" width="880" />
</p>

Four and a half months from `chore: init` to a ledger that reads 83/83 against GNU's own corpus — with the ecosystem wall (bashdb, ble.sh, oh-my-bash, bash-completion, nvm, git's own `t/test-lib.sh`) pinned as CI fixtures behind it. Both core maintainers are upstream contributors to Microsoft's Coreutils-for-Windows project ([microsoft/coreutils](https://github.com/microsoft/coreutils)), vcpkg, and winget-pkgs.

## Architecture

```
src/
├── bin/             The `rubash` reference CLI
├── lexer/           Tokenizer (quoting, escaping, heredocs, continuations)
├── parser/          Recursive-descent (simple cmds, pipelines, case, arith-for, [[ ]])
├── expand/          Expansion engine (parameters, glob, braces, tilde, arith)
├── executor/        Command execution, builtins, dispatch, arrays, traps, identity
├── builtins/        62 engine builtins (declare, read, printf, kill, ...)
├── fd/              fd table and transports
├── jobs/            Job control, coprocs, process substitution
├── shell/           ShellState, subshell clone, init
└── lib.rs           Core types and error handling
```

- **Lexer**: Bash-style quoting, escaping, comments, variables, command substitution, arithmetic expansion, here-doc/here-string tokens, common redirects.
- **Parser**: Simple commands, pipelines, AND/OR lists, functions, brace/subshell groups, `if`, `for`, arithmetic `for`, `while`, `until`, `case`, `select`, `[[ ... ]]`, `coproc`, `time` prefixes.
- **Executor**: External commands, pipelines, redirects, temporary assignments, function calls, `source`/`.`, `eval`, shebangless script fallback, and the per-child argv dialect contract (shell spellings for POSIX-layer children, classified Win32 translation for native ones — `docs/windows-path-contract.md`).
- **Expansion**: Variables, positional parameters, indexed and associative arrays, command substitution, arithmetic expansion, brace expansion, tilde expansion, pathname globbing, `${parameter...}` operators, case/replacement transforms.
- **Builtins**: 62 engine builtins — the machine-checked list lives in [`docs/builtins.md`](docs/builtins.md) (kept in sync by a test over `src/executor/builtin_names.rs`), plus intentionally-hidden fast-path builtins (`sleep`, `dirname`, `basename`, `uname`, `arch`) that keep `type`/`compgen -b` reporting them as external, exactly like GNU.

### Subshells without fork

POSIX `fork()` has no Win32 equivalent. Emulation layers (MSYS2, Cygwin) fake it at the syscall level — expensive, fragile, and the source of their best-known quirks. Rubash reproduces fork's *semantics* instead, at three layers:

1. **In-process subshells.** `( list )` and `$( )` never spawn a process. `ShellState::clone` produces the child's variables, aliases, functions, traps, and history — the memory side of a fork — and the copy is discarded when the subshell ends.
2. **A real-handle fd table with POSIX `dup` semantics.** Slots hold raw Windows `HANDLE`s, and `DuplicateHandle` duplicates share the same kernel file object — therefore the same file offset. That is exactly POSIX "dup shares the open file description", verified empirically in a POC before landing. `fork_table()` duplicates the whole table handle-by-handle, the way `fork` copies the fd table but not the file objects.
3. **Real processes only at true process boundaries.** External commands and pipeline members run via `CreateProcess` + `os_pipe` (native `fork`/`exec` on Unix); background jobs and coprocs get their own processes, with job control on Job Objects and SIGCONT via `ResumeThread` on Windows.

The result: subshells and command substitutions pay zero process-creation cost, while everything a script can observe — exit codes, fd inheritance, shared offsets, signal dispositions — behaves like GNU Bash.

## Identity and Compatibility (rubash#154)

The engine presents a **selectable platform identity** instead of outsourcing it to whatever `uname.exe` happens to sit on PATH. `uname` and `arch` are engine builtins (full option parsing, coreutils/MSYS2 output shapes), and `OSTYPE`/`MACHTYPE`/`HOSTTYPE` follow the same persona. Ecosystem scripts that branch on `case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*)` or on `$OSTYPE` take their best-tested path instead of exiting — the identity exists because real scripts (bash-completion among them) refuse to run under an identity they don't recognize.

- **Default persona: MSYS2-compatible** (Windows default; disclosed everywhere — this is a compatibility mask, not a claim of being an MSYS2 port. Non-Windows builds always report the honest-native identity — the MSYS persona exists to keep Windows inside the MSYS/Cygwin script ecosystem and is unreachable elsewhere):
  - `uname -s` → `MSYS_NT-<ver>` — or `MINGW64_NT-` / `UCRT64_NT-` / `CLANG64_NT-` … when `MSYSTEM` is set, mapping the value the same way the MSYS2 runtime does;
  - `uname -m` / `arch` → `x86_64` / `aarch64` (build arch);
  - `uname -r` → the Windows version string (`10.0-19044`, same string that finishes `uname -s`);
  - `uname -o` → `Msys`; `uname -a` → `sysname nodename release version machine Msys` (the Git Bash shape);
  - `OSTYPE=msys`, `MACHTYPE=<arch>-pc-msys` (bound `set_if_not`-style, as GNU variables.c:723-725 does — an inherited value wins).
- **`RUBASH_IDENTITY=native` switches to the honest-native persona**: `uname -s` → `Windows_NT` (the native `%OS%` value), `uname -o` → `Windows`, `OSTYPE=windows`, `MACHTYPE=<arch>-pc-windows`. Tests and native-first users select this; unset the variable (or set any other value) to return to the default.

**Disclosure surfaces** (the persona must never be silent):

- `rubash --help` prints an Identity section (current persona + how to switch);
- `rubash --identity` prints the persona and every effective value (`uname -s/-m/-r/-o`, `arch`, `OSTYPE`, `MACHTYPE`);
- this README section and the repo-root `SKILL.md` disclosure section (so AI agents consuming the repo also know the persona semantics).

`uname`/`arch` are *hidden* fast-path builtins (like `sleep`/`dirname`): `type`/`enable`/`compgen -b` keep reporting them as external commands, and they work even when PATH carries no `uname.exe` at all.

## Testing

```bash
# Unit + integration tests (906 integration test files under tests/, as of 2026-10-30)
cargo test --lib
cargo test --test cli_tests

# Regression suite: GNU goldens, ecosystem snapshots, perf canaries
cargo test --test regression

# bashdb compatibility
cargo test --test cli_tests bashdb_compat -- --nocapture

# Source expansion
cargo test --test cli_tests source_expands -- --nocapture

# Console-less session gate (single hidden console, zero visible windows)
cargo test --test console_window_gate
```

The engine also runs the [bashdb](https://github.com/Trepan-Debuggers/bashdb) core debugger loop (list, step, next, where, continue, quit) end-to-end.

Compatibility is defended four ways, all of them mechanical:

- **GNU's own corpus** — the 83-suite upstream test suite, replayed in CI on every push and re-measured in full on the ledger cadence above.
- **An independent differential suite** — 246 hand-written cases run against a locally built GNU Bash 5.3.20 oracle, comparing stdout, stderr, and exit code byte-for-byte: **244/246** on current master (222/246 at the first audit, 2026-10-22; residuals tracked as issues).
- **The regression matrix** — every fixed issue lands with a regression test in `tests/` named for its issue, asserting the GNU behavior, in the same change as the fix. Fixes without a pinned test do not count as done.
- **Ecosystem fixtures** — the real-world corpora in the table above (oh-my-bash, bash-completion, nvm, git's test framework, …) pinned as CI fixtures, so a semantic change that breaks the ecosystem fails the build, not a user.

## Documentation

- GitHub issue tracker — **single source of truth** for Rubash ↔ GNU Bash compatibility status (snapshot ledgers archived 2026-10-10 in issue #477)
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — provenance statement: what Rubash is relative to GNU Bash, and contributor methodology rules
- [`docs/builtins.md`](docs/builtins.md) — builtin inventory and dispatch model
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture setup and smoke test
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — how to run GNU Bash upstream tests

## Development Principles

- Fix by root cause subsystem, not by individual expected-output lines.
- Keep bashdb external and clean; temporary instrumentation is for diagnosis only.
- Every failing bashdb command is an opportunity to find and fix a Rubash compatibility gap.
- Compatibility baseline is GNU Bash 5.3.x — the vendored upstream tree (`third_party/bash`) is at the Bash-5.3 patch 20 level (#957), with local and WSL GNU 5.3.20 baselines as differential oracles (#978).

## Provenance

Rubash is a **from-scratch rewrite of GNU Bash semantics in Rust** — not a port or translation of the GNU Bash C code. No GNU Bash source is compiled into, linked with, or copied into Rubash's own code. Compatibility is defined against the *observable behavior* of GNU Bash 5.3.x and verified by black-box differential testing. The vendored GNU Bash source lives in the separate `third_party/bash` submodule under its original GPL-3.0-or-later license and serves only as a semantic reference and test oracle. Full statement and contributor rules in [`docs/PROVENANCE.md`](docs/PROVENANCE.md).

## License

MIT — see [`LICENSE`](LICENSE).

## Contributing

Issues, compatibility reproductions, focused regression tests, and implementation patches welcome. Read [`AGENTS.md`](AGENTS.md) before compatibility work.

## Acknowledgements

- GNU Bash team — the reference implementation whose observable behavior defines our compatibility target
- Trepan-Debuggers/bashdb — external debugger and compatibility stress test
- Rust community — language and tooling

---

*Last updated: 2026-10-30*
