# GNU Bash Upstream Tests

This repository tracks the official GNU Bash source tree as a Git submodule at:

```text
third_party/bash
```

The Bash conformance-style tests live in:

```text
third_party/bash/tests
```

## Why a Submodule

GNU Bash does not publish the test suite as a separate repository. The tests are
part of the main Bash source tree, so a submodule gives us:

- a pinned upstream commit for reproducible test runs;
- clear provenance for GPL-licensed upstream material;
- a simple update path when we want to move to a newer Bash revision.

Do not copy the `tests/` directory into this repository unless there is a strong
reason to fork individual tests.

## Initialize

```sh
git submodule update --init --depth 1 third_party/bash
```

If the submodule commit changes, use:

```sh
git submodule update --init third_party/bash
```

## Running Strategy

Bash upstream tests are driven from `third_party/bash/tests` with `run-*` scripts
and the `THIS_SH` environment variable. For example, upstream drivers expect a
shell that can run script files:

```sh
THIS_SH=/path/to/shell sh run-test
```

Use the project runner instead of invoking upstream scripts directly:

```sh
scripts/run-bash-upstream-tests.sh
```

On Windows, run the harness from an environment that can build Rubash and can
execute the upstream Bash `run-*` drivers. In practice this usually means:

- initialize the Visual Studio/MSVC build environment first, or use a Developer
  Command Prompt, so the `cargo build` inside the runner can link;
- use Git Bash as the driver shell; if `bash` is not on `PATH`, call it by
  absolute path.

Example from a Windows host where Git for Windows is installed in the default
location:

```sh
winuxsh -c 'cd C:/path/to/rubash && BASH_UPSTREAM_STRICT=1 C:/Progra~1/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh'
```

Replace `C:/path/to/rubash` and the Git Bash path for your machine. The short
`C:/Progra~1/...` form avoids quoting the `Program Files` space through nested
shells.

For focused debugging, pass one upstream runner name after the script:

```sh
winuxsh -c 'cd C:/path/to/rubash && BASH_UPSTREAM_STRICT=1 C:/Progra~1/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-minimal'
```

## Safety Model

Do not run upstream Bash `tests/run-*` scripts directly from a user directory.
Some upstream tests intentionally create and remove files in their current
working directory, including broad glob deletes. Always use
`scripts/run-bash-upstream-tests.sh`.

The project runner is intentionally defensive:

- it derives the repository root from the runner script location and refuses to
  run if that root is `/`, `$HOME`, `$HOME/Desktop`, `$HOME/Downloads`, or
  `$HOME/Documents`;
- it verifies the root looks like this repository by requiring `Cargo.toml`,
  this runner, and `third_party/bash/tests`;
- it creates one isolated work directory per upstream runner under
  `target/bash-upstream-tests/work/`;
- its own cleanup path uses a guarded recursive delete that refuses to delete
  anything outside `target/bash-upstream-tests/work/`;
- it runs the shell under test with isolated `HOME` and `TMPDIR` directories
  inside the per-runner work directory;
- it shadows `rm`, `touch`, `mkdir`, `cp`, `mv`, and `ln` with wrappers that
  refuse to operate from or on paths outside the per-runner work directory.

These checks are part of the test harness contract. Changes to the runner must
preserve the property that a bad working directory, a bad `HOME`, or an upstream
test containing destructive commands cannot modify the developer's real home
directory.

The runner copies `third_party/bash/tests` into a temporary per-test worktree
under `target/bash-upstream-tests/work/` before running each upstream `run-*`
script. This is required because the upstream tests create and delete files in
their working directory.

The runner writes:

- `target/bash-upstream-tests/summary.md`
- `target/bash-upstream-tests/results.tsv`
- `target/bash-upstream-tests/logs/*.log`
- `target/issue-suites/results/bash-upstream-tests/<run-id>/<runner>/` containing the unfiltered runner log, generated test workspace, expected files, and temporary output

The raw artifact directory is intentionally separate from the progress table so a later focused run cannot erase evidence from an earlier run. Set `BASH_UPSTREAM_RUN_ID` when a stable, externally supplied artifact name is required.

By default ordinary upstream compatibility differences are non-blocking and the
progress run exits successfully even when tests fail. Set
`BASH_UPSTREAM_STRICT=1` to make any upstream failure fail the command. Every
runner (one upstream test file/driver) has a 60-second timeout by default; override it with
`BASH_UPSTREAM_TIMEOUT`. A timed-out runner receives a follow-up kill after
`BASH_UPSTREAM_KILL_AFTER` seconds (default 5), so child processes do not remain
attached to the next test. A timeout always fails the command, even when strict
mode is disabled. The runner validates `BASH_UPSTREAM_TIMEOUT_BIN` with a
short probe and rejects simplified timeout stubs unless they return GNU
timeout status `124`.

## Windows Troubleshooting

If many or all runners fail with exit `126`, inspect one log under
`target/bash-upstream-tests/logs/`. A message like this means the harness
safety guard rejected a path before Rubash actually ran the Bash test:

```text
Refusing rm outside Bash upstream work dir: /c/.../work/run-alias/tests
Allowed: C:/.../work/run-alias
```

That is a path-format mismatch between Git Bash `/c/...` paths and Windows
`C:/...` paths, not a shell compatibility failure. The guard exists to keep
upstream tests from deleting files outside the isolated work directory. Do not
remove or weaken it; preserve normalization so `/c/...` and `C:/...` compare as
the same path.

`run-minimal` may print both `Testing /c/.../rubash-wrapper` and
`Testing C:/.../rubash-wrapper`. The runner filters both forms; if the test
exits `0` but only one of those banner lines remains in
`*.unexpected.log`, update the filter rather than treating it as a Rubash
semantic failure.

If the command fails before tests start because `bash` is missing, use an
explicit Git Bash executable as shown above. If it fails during `cargo build`,
rerun from a Visual Studio Developer Command Prompt or initialize `vcvars64.bat`
before starting the harness.

Current local baseline:

| Environment | Total | Passed | Failed | Pass rate |
|-------------|-------|--------|--------|-----------|
| Windows + Git Bash full upstream run | 87 | 87 | 0 | 100.00% |

This table is the `.right` expectation-file runner baseline. It is intentionally
separate from the actual-output comparison against native Bash. The current
authoritative actual-output status is maintained in the rubash README "Compatibility at a Glance" ledger (snapshot archive: issue #477; `docs/COMPATIBILITY-STATUS.md` was deleted 2026-10-10).

The runner stays non-strict for ordinary compatibility differences in CI so it
can serve as a progress signal, but timeout failures and invalid timeout
implementations are always blocking. The current local baseline passes the full
upstream runner set that ships in the submodule. When Bash adds new `run-*`
scripts or the submodule advances, re-run the suite and update this table.