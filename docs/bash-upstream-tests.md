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

By default the upstream progress run is non-blocking and exits successfully even
when upstream tests fail. Set `BASH_UPSTREAM_STRICT=1` to make any upstream
failure fail the command.

Current local baseline:

| Environment | Total | Passed | Failed | Pass rate |
|-------------|-------|--------|--------|-----------|
| Windows + Git Bash full upstream run | 87 | 87 | 0 | 100.00% |

The runner still stays non-strict in CI so it can serve as a progress signal,
but the current local baseline passes the full upstream runner set that ships in
the submodule. When Bash adds new `run-*` scripts or the submodule advances,
re-run the suite and update this table.
