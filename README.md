# Rubash

An embeddable, cross-platform GNU Bash-compatible shell engine, written from scratch in Rust. Windows-first, with native Linux and macOS builds.

[中文](README.zh-CN.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Rubash ships as an embeddable, headless engine — lexer, parser, expansion, executor, and builtins — targeting GNU Bash 5.3.0 observable behavior. It is not a shell product: the repo includes a reference CLI (used by the compatibility harness and tooling), and interactive shells such as niubash embed the engine for all bash semantics while owning line editing, prompts, and completions themselves.

<!-- TODO(golden-lane, ci490): when the last upstream-slice snapshot re-record lands in CI, replace this comment with the headline line:
**83/83 GNU goldens passing** — the full upstream corpus, byte-for-byte (ledger: #477).
Current measured state is 82/83; do not promote early. -->

## The Evidence

### It sources real-world scripts

The engine sources these targets the way a shell would. Every number is a byte-level measurement, and every row links the issue that recorded it.

| Real-world target | Measured result | Issues |
| --- | --- | --- |
| nvm.sh v0.40.x, full source | loads byte-clean | #130 #155 #162 #169 |
| bash-it, whole repo | 221/221 components load; full framework loads end-to-end | #316 #161 |
| oh-my-bash, repo + theme matrix | 22 libs load, 252/252 functions environment parity, PS1 byte-identical (5 themes) | #143 #148 #149 #160 |
| bash-completion 2.18.0 | 452/452 matrix; `bash -n` canary byte-identical | #138 |
| git-completion | 140/140 | #142 |
| modernish capability suite | 166/166 rc-vector identical | #477 |
| FFmpeg / OpenSSH / PHP / ltmain / cmake configure | `--help` and error-path output byte-identical | #477 |
| git's own test framework (t/test-lib.sh) | runs end-to-end under the engine | #477 |

These results are pinned as regression fixtures (`tests/regression/`, 24 tests incl. GNU-golden matrices, suite-slice snapshots, and perf canaries) running in CI on every push, with a C-source line-level audit ([`docs/SOURCE-AUDIT.md`](docs/SOURCE-AUDIT.md), 336 behaviors inventoried) finding and closing gaps before suites hit them.

### Against GNU Bash's own corpus

The compatibility baseline is GNU Bash 5.3.0's upstream test suite — 83 files, true-baseline measurement: no upstream-script stubs, per-suite TMPDIR, foreground timeouts, environment-bound diffs zeroed after line-by-line audit.

```
PASS (0 diff):   82 suites
DIFF (residual):  1 suite — nameref, one audited line (a coproc
                  reap-timing race GNU itself exhibits)
```

That is up from 3427 raw diff lines on Sep 9, a reduction of more than 99%. The full ledger and inventory are archived in [#477](https://github.com/unixwin/rubash/issues/477); the fully passing suite list lives in the ledger rather than this README.

## Performance

Measured with `scripts/run-perf-suite.sh` (checked-in probes under `benchmarks/`, median wall clock against GNU Bash 5.3.0). Per-probe data lives in the linked issues.

- Array append was O(n²). The amortized-growth fix takes `a+=(i)` at n=3000 from 771 ms to 52 ms, a 14.8x speedup (#437).
- nvm.sh parsing: 852x GNU's time at the first baseline (#241); single-digit multiples after the parse and incremental-reader rounds (#241, #281).
- Near parity: command substitution 1.2x, external process spawn 0.9-1.5x of GNU (#242).
- Interpreter-loop hot paths (read loops, expansion, function calls) still trail GNU. The open per-probe numbers and the attack queue are in #241 and #242.

## Platforms

| Platform | Status |
| --- | --- |
| Windows x64 (primary) | Full stack, one self-contained binary speaking Win32, MSYS2-compatible identity for the bash ecosystem; GNU regression goldens and canaries run on Windows runners |
| Linux `x86_64-unknown-linux-gnu` | Engine and test suites run in CI; release tarball; real getrlimit/chmod/faccessat/uname(2) semantics and signal delivery; GNU corpus measured 19/24 byte-identical on the native Linux binary (WSL-internal run) |
| macOS `aarch64-apple-darwin` | Library tests execute on macOS runners; cross-target check green; coreutils-correct uname arms; release tarball |
| Android `aarch64-linux-android`, `armv7-linux-androideabi` | Cross-compile check legs in CI (check-only today, no NDK linking) |

The engine's semantic model (in-process subshells, fd-table semantics, process boundaries) is deliberately platform-neutral, so the same ledger travels. Windows remains the primary target and the platform with the full stack; per-platform gaps are tracked in [`docs/cross-platform-gap-ledger.md`](docs/cross-platform-gap-ledger.md).

## Quick Start

### Build from Source

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

Windows has the full stack today. The engine also builds and runs on Linux and macOS (see [Platforms](#platforms)).

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

Engine tests: `cargo test --lib` (full matrix in [`docs/architecture.md`](docs/architecture.md)).

## Identity

`uname`, `arch`, `OSTYPE`, and `MACHTYPE` are engine builtins backed by a selectable persona: MSYS2-compatible by default (disclosed everywhere as a compatibility mask, not a port claim), or honest-native Windows via `RUBASH_IDENTITY=native`. `rubash --identity` prints the active persona and every effective value. Full semantics: [`SKILL.md`](SKILL.md) (rubash#154) and [`docs/platform-identity.md`](docs/platform-identity.md).

## FAQ

### Does Rubash require WSL?

No. Rubash is a single native binary; on Windows it speaks Win32 directly, with no POSIX emulation layer and no WSL dependency. WSL appears only in the development workflow, as the harness that runs GNU Bash 5.3.0 as the measurement oracle.

### Is the compatibility measured or claimed?

Measured: 82 of GNU Bash 5.3.0's own 83 upstream test suites match byte-for-byte today, and every residual diff line is individually audited (see [The Evidence](#the-evidence) and #477).

### Can I embed it?

Yes. The engine ships as a Rust library crate (lexer, parser, expansion, executor, and builtins run in-process), and the `rubash` CLI is only the reference binary. niubash embeds it for all bash semantics.

## Documentation

- Issue tracker — single source of truth for compatibility status; measurement ledgers archived in [#477](https://github.com/unixwin/rubash/issues/477)
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — provenance statement: what Rubash is relative to GNU Bash, and contributor methodology rules
- [`docs/architecture.md`](docs/architecture.md) — subsystems, fork-free subshell design, engine testing
- [`docs/platform-identity.md`](docs/platform-identity.md) — persona table, argv/path dialect contract, native-vs-MSYS comparison
- [`docs/cross-platform-gap-ledger.md`](docs/cross-platform-gap-ledger.md) — per-platform gap tracking
- [`docs/builtins.md`](docs/builtins.md) — builtin inventory and dispatch model
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — how to run GNU Bash upstream tests
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture setup and smoke test

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

*Last updated: 2026-10-10*
