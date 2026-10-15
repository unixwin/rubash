# Debugging Rubash Scripts with External bashdb

This note documents the supported workflow for running a clean external bashdb
against scripts executed by `target/debug/rubash.exe`.

## Current Status

Rubash can now run the clean bashdb command loop well enough for the core
debugger workflow:

- `list` shows the debugged script source.
- `step` enters shell functions.
- `next` advances to the next source line.
- `where` prints a stack trace.
- `continue` runs the debugged script to completion.
- `quit` exits the debugger.

Boundary status (2026-08):

- `info files` uses one canonical `/d/...` source identity on Windows.
- Numeric source breakpoints such as `continue 4` stop at the requested source line.
- `info functions`, `info functions foo`, `list foo`, `info files`, and `info variables` without filters have been exercised.
- `shell --no-vars --no-fns` works. `shell --shell /usr/bin/bash --norc` still produces `--init-file: command not found`; native Bash reproduces the same clean bashdb failure, so this is not currently a Rubash fix target.
- `info variables -a` and `info variables -A` now return indexed and associative variables; the declaration option path is covered by regression tests.
- Sourced-file command errors now retain the sourced filename and line number, while `$0` remains the top-level script name.
- A bounded command matrix covers `help`, `break`/`clear`, `display`, `eval`, `set`/`show`, `info files`, `info functions`, and `info variables`. `watch` requires a variable that exists in the current frame; clean bashdb has no top-level `unwatch` command. `restart` with a relative Windows launcher path remains a separate path-resolution edge case.

The verified fixture is the clean bashdb checkout under:

```sh
target/bashdb-clean/
```

The verified launcher is:

```sh
target/bashdb-clean/bashdb-generated
```

## What "Launcher Path" Means

The launcher is the top-level bashdb shell script that the user executes. It is
not part of Rubash. In the local fixture it is named `bashdb-generated`.
A normal bashdb install may name the same top-level script `bashdb`.

That script must be able to find the bashdb library directory. The library
directory is the directory containing files such as:

```sh
bashdb-part2.sh
dbg-main.sh
init/
lib/
command/
```

In the local fixture, the generated script has `_Dbg_libdir` pointing at
`target/bashdb-clean`. If a user moves the checkout or uses a different clean
bashdb tree, either regenerate the launcher for that location or pass the
library path explicitly with bashdb's `-L` / `--library` option.

## Quick Start with the Local Fixture

Build Rubash first:

```sh
cargo build
```

Run bashdb under Rubash:

```sh
export TERM=xterm DARK_BG=0
target/debug/rubash.exe target/bashdb-clean/bashdb-generated --no-highlight target/bashdb-probe-target.sh
```

For non-interactive smoke testing:

```sh
export TERM=xterm DARK_BG=0
printf 'list\nstep\nnext\nwhere\ncontinue\nwhere\nquit\n' | \
  target/debug/rubash.exe target/bashdb-clean/bashdb-generated --no-highlight target/bashdb-probe-target.sh
```

A successful smoke test exits `0`, prints the target script source for `list`,
steps into the `foo` function, prints the stack for `where`, and has empty
stderr.

## Using a Fresh Official bashdb Checkout

A user should not need to patch bashdb for this workflow. The needed fixes are
in Rubash compatibility behavior. The repository keeps the small target-script
fixtures under `tests/fixtures/bashdb`; only the generated launcher and staging
copy belong under ignored `target/`.

To recreate the local fixture after a fresh checkout, first obtain/build an
official bashdb checkout, then run:

```sh
bash scripts/setup-bashdb-fixture.sh /path/to/bashdb-checkout-or-install
```

The script copies the tracked probe inputs and the built bashdb launcher into
`target/`. It preserves an official checkout's `getopts_long.sh` and uses the
tracked fallback only when that runtime file is absent. It fails clearly when the
external checkout or launcher is missing; it does not silently claim that bashdb
tests are available. The currently verified external source is
`Trepan-Debuggers/bashdb`, branch `bash-5.2`, commit
`f139cc23183798cb0874358a3d624850b418266c`, release `5.2-1.2.0`.

For a fresh bashdb checkout, the required user work is:

1. Check out a bashdb version compatible with Bash 5.2, for example the
   upstream `Trepan-Debuggers/bashdb` `bash-5.2` branch.
2. Generate or install bashdb normally so there is a top-level launcher script.
3. Confirm the launcher can locate its library directory, or pass it explicitly
   with `-L /path/to/bashdb-libdir`.
4. Invoke that launcher through Rubash:

```sh
target/debug/rubash.exe /path/to/bashdb --no-highlight /path/to/script.sh
```

If using a source-tree launcher whose built-in paths are wrong, use:

```sh
target/debug/rubash.exe /path/to/bashdb --library /path/to/bashdb-libdir --no-highlight /path/to/script.sh
```

The `bashdb-libdir` path should be the directory that contains `dbg-main.sh`,
`bashdb-part2.sh`, `init/`, `lib/`, and `command/`.

## User-Facing Expectations

For someone using the checked-out fixture in this repository, the workflow is
one build plus one command. They do not need to understand Rubash internals.

For someone bringing their own official bashdb checkout, the only extra concept
is the launcher/library split:

- Run the bashdb launcher script through Rubash.
- Ensure the launcher finds the bashdb library directory.
- Use `--no-highlight` for the most stable non-interactive output on Windows.

## Full bashdb Coverage Goal

The current verified state is the core debugger loop: startup, `list`, `step`,
`next`, `where`, `continue`, and `quit`. This is enough for useful script-level
compatibility debugging, but it is not a claim that every bashdb command and
interactive feature has been certified.

The development goal is broader: bashdb should become a full external stress
program for Rubash. New failing bashdb commands, completion paths, breakpoint
features, history paths, restart flows, or interactive edge cases should be
triaged as Rubash compatibility gaps first. Do not patch bashdb to hide those
failures unless the failure is proven to be a local fixture or launcher setup
problem.

## Regression Coverage

The CLI tests sync the tracked probe target and, when absent, the tracked
`getopts_long.sh` fallback into `target/`. They do not overwrite an installed
external getopts implementation. Pure shell-language regressions in
`bashdb_compat` run without a bashdb installation.

Only tests that invoke `target/bashdb-clean/bashdb-generated` return early when
that external launcher is absent, printing `SKIP` with the setup command under
`--nocapture`. Rust's test harness counts these early returns as passed, not
ignored: they are **not evidence of bashdb coverage**. Once the launcher exists,
the tests run their original assertions; launcher/library/runtime failures are
not skipped. The two existing stdin-loop ignores remain unchanged.

The dedicated fixture gate must require the external launcher explicitly:

```sh
sh scripts/check-bashdb-fixture.sh
```

With this setting, a missing launcher fails rather than returning early. A
normal suite run without it may skip external tests and is not a bashdb
coverage gate. Existing launchers still run the original debugger assertions;
the switch does not certify launcher/library validity or suppress failures.
The script exports `RUBASH_REQUIRE_BASHDB_FIXTURE=1` before invoking Cargo;
the equivalent explicit command is
`RUBASH_REQUIRE_BASHDB_FIXTURE=1 cargo test --test cli_tests --locked bashdb_compat -- --nocapture`.
Run it from the repository root with an outer timeout appropriate for the
fixture host, and use the forwarded Rust test arguments for a smaller slice.

The requirement policy has an independent regression target:

```sh
cargo test --test issue615_bashdb_fixture_gate --locked -- --nocapture
```

The focused regression suite for this workflow is:

```sh
cargo test --test cli_tests bashdb_compat -- --nocapture
```

It covers the Rubash semantics bashdb depends on: source expansion, command
word splitting, quoted `$@`, arithmetic commands, multiline arithmetic-for
headers, DEBUG/EXIT trap behavior, `/dev/stdin`, `tty`, dynamic fd redirects,
Windows drive paths, `enable -a`, escaped backticks through `eval`, and command
substitution output capture.
