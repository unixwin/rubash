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
| Windows + Git Bash full upstream run | 86 | 0 | 86 | 0.00% |

Rubash currently does not provide enough Bash-compatible behavior to pass the
full upstream suite. Before making these tests strict in CI, implement or adapt:

- executing a script file passed as argv;
- `-c` command execution;
- preserving stdout, stderr, and exit status for golden-output comparison;
- a curated allowlist of upstream `run-*` tests matching implemented features.

Start with parser, quoting, redirection, and simple builtin tests. Add variable
expansion, pipelines, control flow, and job-control tests only after those
features are implemented.
