# Issue 626: restricted library invocation

## Root cause and scope

At baseline `66819d41`, `ShellInvocation` put both `-r` and
`--restricted` into the named `shell_flags` list. `apply_to_executor`
validated that list against `set -o` options, which correctly excludes
`restricted`, and rejected the invocation before executing its command.

Keep restriction in a dedicated invocation field instead. Do not add it to
`SHELL_OPTIONS`, `SHELLOPTS`, or the `set -o` listing. Explicit
`-o restricted` and `+o restricted` must still fail validation. Both short
option parsing paths preserve combined flags and reject cancelling an
already requested restriction with `+r`.

This changes only library invocation parsing/application. It does not change
the native executable's startup driver or the runtime meaning of `set -r`.

## GNU specification and startup boundary

The authoritative source is the existing GNU submodule in
`D:/repo/rubash/third_party/bash`:

- `flags.c:182` flag table: `r` is an independent flag.
- `flags.c:226 change_flag`: rejects disabling an already enabled `r`.
- `flags.c:277 change_flag`: calls `maybe_make_restricted` only after
  `shell_initialized` for a runtime flag change.
- `shell.c:679-687 main`: saves invocation restrictions and clears them
  before startup files.
- `shell.c:722 main`: runs startup files.
- `shell.c:738-744 main`: restores restrictions after startup files.
- `shell.c:1278 maybe_make_restricted`: marks PATH, SHELL, ENV, BASH_ENV,
  and HISTFILE read-only.

WSL `/usr/local/bin/bash` reports `5.3.0(1)-release`. Script-file probes
confirm startup `$-` lacks `r`, `cd .` succeeds, and PATH is writable;
the command body has `r`, rejects `cd`, and never lists a `restricted`
named option. `-r +r` and `-o restricted` exit 2.

Embedders must use an explicit startup boundary:

```rust
invocation.apply_to_executor_before_startup(&mut executor)?;
run_host_startup_files(&mut executor)?;
invocation.finish_startup(&mut executor);
execute_user_command(&mut executor)?;
```

`apply_to_executor` remains the immediate application convenience for
startup-free callers. It must not be used before host-owned startup files.
`finish_startup` never clears a restriction enabled by a startup `set -r`.

**Downstream integration remains required:** Niubash currently calls
`apply_to_executor` before its startup dispatch (`src/main.rs:510` and the
`-C` path at `src/main.rs:1155`). Merely updating the rubash dependency would
remove the rc=2 rejection but restrict those startup files too early. The
captain must migrate both host call paths to the two-phase API, activating
after the selected startup chain and before user commands/hooks. This
worktree does not modify Niubash or claim an end-to-end host timing fix.
Do not close the overall issue based only on this library patch.

## Verification (Windows, 2026-10-15)

Raw artifacts: `target/issue-suites/results/issue-626/`.

- `baseline-library.txt`: the unmodified baseline invocation module,
  compiled against the same Executor, rejects both flag spellings with
  `restricted: invalid shell option name`.
- `installed-before.txt`: installed Windows Niubash `-r -c 'echo ok'`
  reproduces the rejection and exit 2.
- `oracle.sh`, `startup.sh`, `probe.sh`, `gnu.txt`: bounded WSL GNU probes.
- `cargo test --locked --test issue626_restricted_invocation -- --test-threads=1`:
  8 passed. Tests call the library directly, not the native binary.
- `cargo test --locked --lib invocation::tests -- --test-threads=1`:
  9 passed.
- `cargo test --locked --test issue527_restricted_shell_entry -- --test-threads=1`:
  5 passed.
- `cargo fmt --check` and `git diff --check`: passed.

No user-facing diagnostic strings changed; no translation catalog update
is needed. API documentation and tests are not localized runtime messages.
Only the invocation implementation, regression tests, and this document belong
in the PR. Downstream startup integration remains separate; this PR must not
close issue 626.
