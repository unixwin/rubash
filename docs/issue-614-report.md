# rubash#614 Verification Report

Date: 2026-10-15. Worktree: `D:/repo/rubash-wt-614`, branch `fix/issue-614`, baseline `334afab2`. No rebase or commit performed.

## Changes

- `src/executor/getopts_enable.rs`: stale `OPTIND` now normalizes to `$# + 1` at EOF, matching GNU `sh_getopt` (`third_party/bash/builtins/getopt.c:114-123 sh_getopt`).
- `src/executor/command_prepare.rs`: one whole parameter word owns one array-subscript memo scope; pending arithmetic subscript writes flush before alternate set/null checks. This removes repeated negative-array diagnostics while preserving distinct `${a[i++]}` occurrences. GNU owner: `third_party/bash/arrayfunc.c:1483 array_value_internal`, negative-index handling `1577-1583`.
- `tests/issue614_regressions.rs`: five focused Rubash regressions.
- Niubash `crates/niubash-runtime/src/shell.rs`: history provider is created only at interactive entry; noninteractive `history` remains session-local and does not preload the host file.
- Niubash `crates/niubash-runtime/src/history.rs` and `tests/host_contract.rs`: explicit noninteractive `history -r/-w` remains supported and automatic file loading is interactive-only. GNU owners: `third_party/bash/shell.c:798-812 main`, `shell.c:1845-1855 init_noninteractive`, `bashhist.c:320-347 load_history`.

## Verification

- WSL GNU Bash `5.3.0(1)-release`, invoked from LF script files through `wsl.exe -d Ubuntu-GitLab --exec bash -s`.
- `target/issue-suites/results/issue-614/script-comparison.json`: 11/11 bounded script comparisons match stdout, stderr after script-path normalization, and exit status; includes three upstream `getopts*.sub` slices.
- `cargo test --test issue614_regressions -- --nocapture`: 5/5 passed.
- Existing getopts slice: 6/6 passed; existing array side-effect regression: 1/1 passed.
- Niubash host history contract: 6/6 passed; interactive provider unit test: 1/1 passed.
- `cargo fmt --check -p niubash -p niubash-runtime` and `git diff --check`: passed.

## Remaining

No known #614 semantic remainder. Full workspace suites and the unbounded GNU 83-suite gate were intentionally not run. Existing #609 and merged #620 allocation partial `b404904d` are unchanged. The captain should review and commit; this agent did not commit.

Temporary malformed host files from the earlier WSL argv-path attempt are outside this worktree and were not modified.

## Commands, evidence, and ownership

Oracle/product artifacts live at `D:/repo/niubash/target/issue-suites/results/issue-614/`, not the Rubash worktree target: `oracle.log`, `script-comparison.json`, `verify.log`, `getopts.before.out`, `array.before.err`, and the `.gnu/.rubash/.niu` output files. Driver: `oracle-driver.sh`; product harness: `verify.py`. GNU cases use a 10-second timeout plus 2-second kill grace; product probes use 15 seconds. The array probe changed from 12 diagnostics to 4, matching GNU's four out-of-bounds occurrences. No blanket per-command suppression is used.

Rubash authoritative commands, run in the worktree:

```text
cargo test --locked --target-dir D:/repo/niubash/target/issue-614-rubash --test issue614_regressions -- --nocapture
cargo test --locked --target-dir D:/repo/niubash/target/issue-614-rubash --test executor_tests getopts -- --nocapture
cargo test --locked --target-dir D:/repo/niubash/target/issue-614-rubash --test cli_tests array_subscript_arithmetic_side_effect_updates_variable -- --nocapture
cargo fmt --check
git diff --check
```

Logs in this worktree's `target/issue-suites/results/issue-614/`: `regressions.log`, `executor-getopts.log`, `cli-subscript.log`. The first existing-executor run failed because its hardcoded relative target directory was absent; creating the artifact directory and rerunning passed 6/6.

Niubash commands use a target-local Cargo override pointing at this worktree; no tracked manifest or lockfile changed:

```text
cargo --config target/issue-suites/results/issue-614/cargo-local.toml test --locked --test host_contract history -- --nocapture
cargo --config target/issue-suites/results/issue-614/cargo-local.toml test --locked -p niubash-runtime --lib host_history_provider_is_installed_only_on_interactive_entry -- --nocapture
cargo --config target/issue-suites/results/issue-614/cargo-local.toml test --locked -p niubash-runtime --lib history::tests -- --nocapture
python target/issue-suites/results/issue-614/verify.py
```

History provider regressions: 15/15 passed. Existing foreign-PS1 and framework-PS1 interactive-entry tests each pass; the framework test intentionally emits a missing __git_ps1 diagnostic. No interactive PTY persistence gate was run.

The semantic map records two narrow contracts. Its validator fails on the pre-existing unchanged src/input/readline/ entry because that directory is absent. This unrelated row was not changed. Format and diff checks pass. Process inspection found no rubash.exe; two shared bash.exe processes were left untouched rather than killing other agents' shells.

All three tracked Niubash diffs belong to #614: shell.rs, history.rs, tests/host_contract.rs. They change history initialization/contracts, not #355 performance. Coordinate constructor/interactive-entry hunks with #355 at captain review. No #355 worktree, #609 changes, or #620 changes were edited.
