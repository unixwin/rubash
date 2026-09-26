# Issue Suite DIFF Analysis

## 2026-09-06 Root-Cause Matrix and Implementation Handoff

This checkpoint separates compatibility families by semantic owner; aggregate DIFF counts are not closure criteria. All semantic probes must compare script files with WSL GNU Bash 5.2.21.

### Confirmed and validated

- declare -c: GNU semantics are uppercase first character and lowercase remainder. Assignment, execution, declare -p, +c, and append paths are covered by focused tests.
- Dynamic loop-tail redirection, nounset dynamic-fd close diagnostics, typed PWD/OLDPWD, mapfile, herestr, ifs, and globstar have focused evidence recorded elsewhere.
- Pending lexer changes in src/lexer/continuation.rs and src/lexer/tests.rs are captain-exclusive. The case-pattern ) change passed 29 lexer and 17 parser tests. It still requires a WSL file probe covering nested case, quoted/escaped patterns, extglob, and nested command substitution before submission.

### Open semantic owners

- heredoc: GNU make_here_document (third_party/bash/make_cmd.c:602-627) and parse.y:4563-4567. heredoc3 loses EOF warnings because continuation scanning counts body ); heredoc7 loses nested collection metadata and source locations; heredoc10 loses delimiter/body association across alias expansion. heredoc9 belongs to cprint, not expected-output edits.
- casemod: GNU subst.c:9557-9673; indexed arrays and positional parameters match. Associative ordering differs because GNU iterates hash buckets while Rubash preserves Vec insertion order. Do not sort or reverse without a hash-compatible data model.
- procsub: GNU subst.c process_substitute exposes a consuming pipe. Rubash path-based process substitutions reopen regular temporary files and replay input; same virtual-FD cursor behavior is separate. Fix requires endpoint lifecycle/stream ownership, not offset changes.
- comsub-posix: GNU subst.c:10831-10908 calls chk_arithsub and falls back to command substitution when arithmetic recognition fails. Rubash currently classifies $((...)) irreversibly in parameter_core.rs/embedded_mutations.rs; minimal probe: echo $((echo sh_352.25a);(echo sh_352.25b)).
- invocation: GNU shell.c option plumbing. src/main.rs::run_args is live; basic -i, --rcfile/--init-file, and BASH_ARGV0 wiring now builds cleanly. --pretty-print, full forced-interactive runtime, and complete command-name/rcfile semantics remain open.
- cprint: GNU print_cmd.c; Rust owner src/executor/type_functions.rs. Current output flattens compound AST nodes, duplicates redirects, and formats heredocs incorrectly. Implement recursively; do not change expected outputs.

### Evidence artifacts

- heredoc 调查报告（原 investigation/ 目录，2026-09-26 文档清理删除；git 历史可查）
- target/issue-suites/results/check/heredoc.diff
- target/issue-suites/results/casemod-probe-new.sh
- target/issue-suites/results/casemod-gnu-new.out
- target/issue-suites/results/casemod-rub-new.out
- target/procsub-read-twice.sh
- target/comsub_probe.sh
- target/invocation-probe.sh

### Quoted Parameter Pattern Defect - Fixed (2026-09-05)

The focused tests `quoted_parameter_pattern_glob_chars_are_literal` and `unquoted_heredoc_backslash_and_parameter_errors_match_bash` previously failed identically on clean HEAD: Rubash printed `<*>` and `<*b>` where WSL GNU Bash 5.2.21 printed `<>` and `<b>`. Root cause, confirmed by temporary instrumentation (since removed): the parser passes the pattern to the executor as quoted `'*'`, but `expand_embedded_parameters_preserving_escaped_single_quotes` stripped the single quotes before `decode_parameter_pattern_quotes` ran, so the literal `*` lost its quoted-glob marker (\x11) and became a glob wildcard that matched the empty prefix and removed nothing.

Fix landed in three commits:

- `cc383550`: `src/executor/parameter_patterns.rs::expand_parameter_pattern_word` now masks top-level nested `${...}` behind private slots, decodes pattern quotes so the \x11 marker is injected for literal quoted glob characters, restores the masks, and only then expands embedded parameters. The \x11-to-backslash conversion still applies after a successful decode.
- `823ae0ce`: `src/lexer/dolbrace.rs::scan_braced_parameter` gives a nested `${...}` a fresh quote scope (quote state is pushed, reset on entry, and restored at the closing brace). Without this, an inner pattern quote such as `${v%"${v#?}"}` unbalanced the outer scan and returned `None`, which made the lexer swallow subsequent physical lines into a single word.
- `b8f3a5c3`: the masked-slot index was off by one (`slots.len()` instead of `slots.len() - 1`), leaving a residual marker that defeated nested expansion.

Remaining open in this family: the nested-parameter executor path (`${v%"${v#?}"}` still returning `ab` instead of `a` at the whole-word level) is a separate owner in `src/executor/parameter_core.rs` / `expand_braced_patterns.rs` and is tracked separately; the lexer tokenization fix above is the prerequisite.

Evidence: `cargo test --test cli_tests quoted_parameter_pattern` 2/2, `nested_parameter_expansion_can_supply_pattern_removal` 1/1, `unquoted_heredoc_backslash` 1/1, `cargo test --lib lexer` 30 passed, `cargo test --test parser_tests` 17 passed, `cargo test --test heredoc_regressions` 2 passed, `cargo build` clean. The full `cli_tests` failure name list is unchanged before and after (56 to 54 only because the two pattern tests now pass; `comm` of failure names is empty), so no new regression was introduced.

### Heredoc10 Verification Addendum - Fixed (2026-09-05)

The LF-normalized alias/heredoc probe confirmed a parser-stream mismatch: WSL GNU Bash 5.2.21 treats the lines following a multiline alias definition (`hello`, `world`, `EOF`) as commands and reports `hello: command not found`, while Rubash consumed them as heredoc body text.

Fix landed in `f4edbfde`: the alias replacement re-tokenize path passes `InputOrigin::AliasReplacementDeferredHeredoc` (new `TokenizeOptions` / `tokenize_with_options` API in `src/lexer/mod.rs`; call sites in `src/executor/alias_reparse.rs` and `src/executor/arithmetic_aliases.rs`). The lexer collector emits the heredoc body token for the alias origin but does not advance the physical input line iterator, so the following physical lines stay available for normal command parsing after alias expansion. The default `tokenize()` keeps `InputOrigin::Direct` semantics unchanged, so ordinary heredocs, nested command-substitution heredocs, quoted delimiters, `<<-`, here-strings, and EOF diagnostics are unaffected.

Evidence: the alias heredoc probe now matches GNU on stdout, stderr, and exit status (both emit `after` plus `hello: command not found`, exit 0); the nested heredoc probe matches GNU stdout (`foo bar` / `after`) with matching warning text, line number, and exit 0; `cargo test --test heredoc_regressions` 2 passed; `cargo test --lib lexer` passed; `cargo check` and `cargo build` clean. `src/lexer/continuation.rs` and `src/lexer/tests.rs` were not modified.

### Implementation order

1. Verify and preserve the lexer change, then commit it separately only after WSL file comparison.
2. Finish invocation runtime plumbing and focused CLI probes; keep pretty-print separate.
3. Implement comsub arithmetic fallback and heredoc metadata/alias collection by owner.
4. Implement cprint recursion and separately design pipe-backed procsub endpoints.
5. Re-run each named suite serially, record real counts, and commit only reviewed author/family groups.

> Date: 2026-08-12

> Status refresh: 2026-08-29. The authoritative compatibility status is
> maintained in `docs/COMPATIBILITY-STATUS.md`; the 2026-08-22 attribution
> checkpoint has been retired.
> Do not interpret the historical `.right` runner total as real-output parity.
> Scope: issue #20-#26 compatibility suites, local reruns, and implementation ownership.

## Platform Scope Decision (2026-08-19)

Rubash and Winuxsh are Windows-first products. Before 1.0, complete GNU/Linux
compatibility is explicitly deferred and is not an acceptance gate. Linux CI
failure evidence must still be recorded, but Linux-only expectations should be
platform-gated or marked deferred rather than driving changes that regress
Windows/Winuxsh semantics.

The required release gate is the Windows/Winuxsh focused Rust and integration
test set. Windows path display, native device aliases, signal/mailbox behavior,
coproc behavior, and host integration must remain green. Linux signal, coproc,
and device differences require a separate follow-up when Linux support becomes
a release objective.

This document is the durable version of the issue-suite run notes. Files under
`target/issue-suites/results/` are raw run artifacts; this document is the
tracked summary used to decide what to fix and where.

## Arithmetic Expansion Status (2026-08-24)

The current Windows CLI baseline contains 353 tests: 329 passed and 24 failed. The
previous arithmetic failure `arithmetic_empty_quoted_array_subscript_fails_outside_let`
was Rubash-owned. Arithmetic expansion errors now have an explicit fatality state:
empty quoted array subscripts abort the current command with status 1, while ordinary
arithmetic word errors (`1/0`, invalid octal, and assignment-to-non-variable) report
the diagnostic and continue without `errexit`. Readonly arithmetic in a `case` pattern
continues to preserve the existing nonfatal behavior. Focused regressions and
`cargo check` pass.

After restoring the official bashdb fixture and fixing dynamic braced parameter-name indirection, the focused `bashdb_compat` slice is 35 passed and 12 failed. The earlier 336/19 classification predates fixture restoration and the getopts fix. The current 12 failures are bashdb runtime/command compatibility cases; they must not be classified as missing fixtures. The official launcher smoke now exits 0 and reaches the target, while the remaining debugger failures cover source mapping, nested shell, breakpoint, variable, and command behavior.
The separate aliasconv case remains a reproducible Rubash-owned escaped-quote failure, and the Windows `?.tmp` cases remain filename limitations. Tracked source inputs live under
`tests/fixtures/bashdb/` and can be staged with `scripts/setup-bashdb-fixture.sh`.
The direct `declare -F`/`extdebug` behavior passes when a function is loaded. One full-suite refresh also timed out the sequential coproc test;
two isolated reruns passed in about three seconds, so it is treated as a concurrent
suite-load flake rather than a reproducible coproc defect.

Round 2 isolation: `script_backtick_echo_sed_pipeline_splits_version` still fails
only in the mutable command-substitution AST pipeline. Root cause was the shared shell-sentinel
contract: backtick command-substitution sed arguments retained `\x15`, `\x1f`, and
`\x11` markers after tokenization. The decoder now restores them before sed
filter execution, and both the long script regression and a minimal backtick/`$()`
equivalence regression pass. The quoted positional substitution defect was fixed in
`expand_command_word`: raw metadata for function-shaped `$()` substitutions is
expanded before token-value shortcuts can discard the quoted argument source. Both
`script_sort_pos_params_example_handles_quoted_positional_args` and a minimal
function/`"$@"` regression now pass. A bashdb parameter-replacement case also
exposed `\x1b` quote protection leaking from a quoted positional value; numeric
replacement now strips that internal marker before pattern replacement.
`script_aliasconv_example_converts_aliases` is now fixed in Rubash-owned quote handling.
Words containing command substitution or backticks retain the escaped-apostrophe
sentinel through embedded expansion; final command-word materialization decodes it
after pathname and quote-sensitive processing. Ordinary words keep their existing
quote decoding path. The focused aliasconv regression passes, as does the bashdb
getopts dynamic-indirection regression. No expected-output or upstream bridge file
was changed. A separate lexer regression still leaks `\x11` in a multiline pipeline
single-quoted word and remains open. The raw run is retained at
`target/issue-suites/results/current-cli-full-r2.out`; this classification is not
proof that every bashdb runtime failure is a Rubash semantic defect.

## Pipeline Raw-Byte Transport Slice (2026-08-24)

Objective: make `printf '%s' "$x" | od -An -tx1` byte-exact after a read
record captured raw bytes, closing probes p2/d3/p3/u1/u2/mixed against
GNU Bash 5.2.37.

Root cause: in-process pipeline stages exchanged capture buffers converted
with `String::from_utf8_lossy`, and child stdin/final output writers fed
`.as_bytes()` of those Strings. printf's internal byte buffer carries real
raw bytes (marker-decoded by its raw_bytes writer), so the stage boundary
corrupted them to U+FFFD before `od` ever saw the stream.

Fix: both transport directions now honor the owner-tagged codec contract.
- Producers encode exactly once with `substitution_metadata::bytes_to_shell_text`:
  printf/trap/builtin/function/compound/lastpipe capture sites, external
  child stdout/stderr, host-invoked externals, and cat file reads inside
  `pipeline_exec.rs` / `pipeline_stages.rs`.
- Consumers decode exactly once with the new inverse helper
  `shell_text_to_raw_bytes`: child stdin feeds, final `write_pipeline_output`
  payload, and stderr emission sinks.

Result: all seven byte probes match GNU byte-for-byte (p2, d3, p3, u1, u2,
mixed, v1). Artifacts updated under
`target/issue-suites/results/read-raw-byte-inputs-1/*.pipeline-fix.rubash.out`.

Gates: cargo check clean; unit suite includes new
shell_text roundtrip test (lib substitution_metadata green);
executor_tests full run shows the identical pre-existing failure set as
stashed HEAD (39 failures on both sides; +2 passed are the new regressions)
-- note HEAD itself is not lib-green:
`executor::arithmetic::fatality_tests::invalid_literals_are_fatal_arithmetic_expansion_errors`
fails at 08c2aac3 without local changes and belongs to the arithmetic
owner. cli slices all pass (read 31, heredoc 12, printf 14, mapfile 3,
pipeline 32, c_command 62, coproc 17, trap 9); upstream run-redir and
run-vredir remain 100%.

Newly classified out-of-scope defect found while probing `$()` captures:
an external child spawned for `$(cat file)` cannot open even absolute
Windows-style paths while `head -c` and pipeline-stage children work
(probe c1/c5 vs c6/c3). This is an external-child setup/path family gap,
not byte transport; recorded here as the next gate for that family.
## Read Input Raw-Byte Carrier Slice (2026-08-24)

Handoff objective: close the read raw-byte record gap
(`read x < <(printf '\377\n')`, file and `exec N<file` forms) against
GNU Bash 5.2.37. Probes and raw artifacts:
`target/issue-suites/results/read-raw-byte-inputs-1/` plus working scripts
under `target/probe-rawbytes/`.

Root cause: input redirection targets registered by
`read_io.rs`, `read_redirected_fd.rs`, and the three fd-input sites in
`trap_exec.rs` used strict `fs::read_to_string`. Files produced by
process-substitution materialization contain raw bytes, so every
invalid-UTF-8 record either aborted the whole redirect (`exec 3<binary`
 emitted "stream did not contain valid UTF-8") or dropped the record to an
EOF-style empty assignment.

Fix: added the owner-tagged reader
`substitution_metadata::read_shell_input_file` (bytes -> RAW_BYTE_MARKER
shell text) and routed all five input-file sites through it. Storage,
declare formatting, `$x` expansion, the shared fd table (`TextInput.data`
is already byte-native), and printf's final `raw_bytes` decode now carry
provenance end to end. Pipeless verification matches GNU byte-for-byte:
`printf 'hi\377\n' > f; read x < f; printf '%s' "$x" > out` yields
`68 69 ff` in both shells; `declare -p x` shows the single marker payload;
`exec 3<f` + `read -u 3` / `<&3` no longer error and replay raw bytes into
redirected files.

Gates: cargo check; cargo test --lib executor::substitution_metadata (16);
executor_tests command_chaining::part_047 (35, including three new
raw-byte regressions); part_080 dynamic fd (13) and full slice (156);
cli_tests read (31), heredoc (12), mapfile (3); upstream run-redir and
run-vredir both 1/1 exit 0. semantic map validated after the evidence
update.

2026-08-24 follow-up: the pipeline transport seam is also closed; see the
newer entry below. The command-substitution assignment shape (`p3`) now
matches GNU through the pipeline replay as well.
- coproc read record conversion (`read_io.rs` coproc stdout lossy).
- echo/other builtin write paths do not decode marker payloads yet.
- external child env/stdin mirrors (`external_setup.rs`) remain text.
## Runner Infrastructure Checkpoint (2026-08-21)

The Bash upstream runner preserves the caller toolchain, validates positive
timeouts, applies a kill-after grace period to each upstream `run-*` driver,
and archives the unfiltered log plus generated workspace under
`target/issue-suites/results/bash-upstream-tests/<run-id>/<runner>/`. CI uploads
this directory so focused runs do not erase issue-triage evidence.

A bounded `run-minimal` smoke completed with runner exit 0 and one classified
output difference: Windows displayed `D:/usr` and `D:/tmp` instead of `/usr` and
`/tmp`, plus a tilde escaping difference. This remains an environment/semantic
diff to triage for #25; expected output was not changed. Shellcheck passes at
warning severity; the remaining info-level SC2016 note is the existing upstream
`TEST_FILE` sed expression. No Rubash semantic owner was modified.

The concrete implementation playbook for future agents is
[`docs/gnu-bash-compatibility-implementation-plan.md`](gnu-bash-compatibility-implementation-plan.md).

The focused `vredir` difference ledger is maintained in
[`docs/vredir-diff-todo.md`](vredir-diff-todo.md). It records the raw artifact
paths, root-cause classification, and executable TODOs for the active fd
slice; this document keeps the broader suite history and checkpoints.

## Continuation Checkpoint (2026-08-14)

This is the current durable handoff. The worktree was clean at the checkpoint:
`master` and `origin/master` both pointed to `55c5daf`, with Bash submodule
`b4608166` (`bash-5.3-16`). No source or test change was made during the
read-only status review; the next code change must begin from the native Bash
`redir`/`vredir` slice below.

### Evidence Rules

- Raw command output belongs under `target/issue-suites/results/`.
- This document records interpretation, root cause, and durable counts.
- The newest dated entry wins when this document, an older summary, and a
  remote Issue title disagree.
- `.right` runner success is not proof of current Bash behavior; compare the
  official `.tests` body against GNU Bash actual output as well.
- A suite result is not complete evidence until its runner setup, exit code,
  stdout, stderr, and timeout behavior are recorded.

### Active Native Bash Slice: `vredir`

Raw evidence is under
`target/issue-suites/results/native-bash-20260814-vredir/`.

| Test | Current observation | Next action |
|---|---|---|
| `vredir4.sub` | Dynamic fd plus closed fd, nameref, and invalid descriptor behavior differs | Split into one operation per probe; trace virtual fd lifetime |
| `vredir5.sub` | Ordered input redirect followed by heredoc fails in Rubash | Verify left-to-right redirect application and heredoc precedence |
| `vredir6.sub` | Current minimal sequence matches (`ok 1`, then `10`) | Keep as regression; do not change `ulimit` speculatively |
| `vredir7.sub` | Array dynamic fd variant fails | Recheck array element expansion and dynamic-fd close state |
| `vredir8.sub` | `varredir_close`/closed dynamic output diagnostics differ | Trace descriptor ownership and close timing |

GNU reference: `third_party/bash/redir.c` and
`third_party/bash/tests/vredir*.sub`. Rubash owners:
`src/executor/redirection.rs`, `src/executor/trap_exec.rs`,
`src/executor/read_io.rs`, `src/executor/read_redirected_fd.rs`,
`src/executor/read_builtin.rs`, `src/executor/external_setup.rs`, and
`src/executor/types.rs`.

The working hypothesis is a virtual fd state/lifetime and ordered-redirection
problem. It must be confirmed with a minimal reproducer before editing; do not
fix these failures by changing expected output or by adding another upstream
script bridge.

### Next Turn Contract

1. Establish Bash and Rubash output for each primitive in `vredir5`, `vredir7`,
   and `vredir8` separately.
2. Read the matching GNU `redir.c` paths for allocation, `F_DUPFD`, close,
   undo, and `varredir_close`.
3. Implement one root-cause fix in the fd/redirection owner and add one Rust
   regression in `tests/executor_command_chaining/`.
4. Run the focused Rust test, then bounded `run-redir` and `run-vredir`.
5. Append a dated result entry here containing command, status, artifacts,
   root-cause conclusion, and remaining failures.
6. Check for stuck `rubash.exe`, `bash.exe`, suite, and Cargo processes before
   ending the turn.

This contract is intentionally narrower than the final compatibility goal,
but it does not redefine that goal: it is the next verified root-cause step.

## 2026-08-14 Semantic Kernel Migration Slice

The semantic map v2 and first kernel owners are now present. The canonical map
is `docs/semantic-ownership.tsv`, checked by
`scripts/validate-semantic-map.sh`; the former file-by-file inventory remains
provenance only. Unreferenced placeholder module files were removed, while
`upstream_scripts` was preserved.

The follow-up placeholder audit found 265 additional comment-only Rust files
outside the active module tree. It classified 58 as duplicate-owner candidates
and 207 as host/deferred. No file was deleted in that audit; the raw report is
`target/rust-placeholder-audit.tsv` and the disposition rule is documented in
`docs/bash-source-map.md`.

`src/executor/fd_table.rs` now owns shell-visible read/write capabilities,
dynamic slot allocation, dup/move/close state, shared text input offsets, and
child materialization. `Executor` keeps the old environment keys as a
compatibility mirror. `src/shell/state.rs` and `src/shell/variables.rs` add
typed shell state and export serialization, and `src/jobs/table.rs` owns job
identity, jobspec resolution, completion retention, and coprocess endpoint
metadata. Background/coprocess registration and completion marking now feed
that table.

Evidence from the bounded verification turn:

| Command | Result | Raw evidence |
|---|---|---|
| `cargo check` | pass | command output |
| `cargo test --lib fd_table` | 3/3 pass | command output |
| `cargo test --test executor_tests command_chaining::part_080::test_exec_dynamic_` | 12/12 pass | command output |
| `cargo test --test executor_tests command_chaining::part_080` | 149/149 pass | command output; ordered stderr copy regression closed |
| `cargo test --lib` | 169/170 pass | command output; existing Windows bare-drive path assertion |
| `run-redir` | 1/1 pass | `target/bash-upstream-tests/logs/run-redir.log` |
| `run-vredir` | 1/1 pass | `target/bash-upstream-tests/logs/run-vredir.log` |
| `run-coproc`, `run-procsub`, `run-jobs` | 1/1 each | corresponding `target/bash-upstream-tests/logs/` files |

The ordered stderr-copy failures are closed by correcting the parser's
internal inherited-stderr redirect: a path inherited from `2>&1` is represented
as an append redirect rather than as a duplicate-fd redirect. Ordered output
state now reads `FdTable` first, with the environment keys retained only as a
compatibility fallback. The next fd gate is external child materialization and
removal of that fallback mirror; `<>` and the native `vredir4/5/7/8` probes
remain separate gates.

### 2026-08-14 Coproc virtual close versus external child materialization

The named-coproc fd lifetime slice now has a focused real regression. The
previous close path treated `C[0]` and `C[1]` as one complete virtual fd and
closed both capabilities when `exec {C[1]}>&-` ran. The corrected owner,
`src/executor/trap_exec.rs::close_dynamic_output_fd`, closes only the output
capability and preserves the coprocess stdout reader until it is explicitly
closed or consumed.

Evidence:

| Command | Result | Raw evidence |
|---|---|---|
| `cargo test --test cli_tests c_command_closing_named_coproc_stdin_fd_produces_eof -- --nocapture` | 1/1 pass | CLI regression output |
| `cargo test --test executor_tests command_chaining::part_080 -- --nocapture` | 152/152 pass | fd/coproc regression output |
| `BASH_RUNNER=D:/Git/bin/bash.exe D:/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-coproc` | 1/1 pass | `target/bash-upstream-tests/logs/run-coproc.log` |
| direct `coproc { cat; }` probe | Bash `read:hello`; Rubash `read:`; both `done` | `target/issue-suites/results/coproc-kernel-20260814/` |

The direct probe is the important boundary evidence. The Rubash-only loop
passes, so virtual writer-close and reader preservation are no longer the
root cause there. An external `cat` still does not receive the same data/EOF
behavior after child fd materialization. This is an open WinuxCmd/std child
setup and `FdTable::materialize_for_child` TODO, not a reason to revert the
virtual fd fix or to add an output bridge.

Open gates:

- [ ] Make materialized coprocess stdin/out handles follow the shell fd
      lifetime for external children.
- [ ] Add external-child `cat` regressions for read, write, duplicate, close,
      and `wait "$C_PID"` status, with dated stdout/stderr/status artifacts.
- [ ] Re-run the official Bash `.tests` `coproc` actual-output body and update
      its ledger row only after the bridge is unnecessary.

### 2026-08-14 Bridge-free coproc inherited-cat streaming

The minimal external-child reproducer exposed a second real boundary. The
coproc child was correctly spawned with inherited process stdin, but its
no-operand `cat` was delegated to `winuxcmd/cat.exe`. In the nested anonymous
pipe path that wrapper did not forward data before EOF, so the parent blocked
on `read <&"${C[0]}"` before it could close `C[1]`.

The Rubash owner `src/executor/external_file_builtins.rs` now handles only the
matching case: no file operand, no explicit stdin/heredoc redirect, and
`INHERIT_PROCESS_STDIN=1`. It reads chunks from the inherited pipe and writes
each chunk through the normal redirected-output path. Explicit fd redirects
continue through `FdTable` and `external_setup`.

Evidence is stored at
`target/issue-suites/results/coproc-stream-20260814-6d83e10/`:

| Command | Result |
|---|---|
| Bash delayed-cat script | rc 0, `read:hello` |
| Rubash delayed-cat script | rc 0, `read:hello` |
| `cargo test --test cli_tests coproc -- --nocapture` | 10/10 |
| `cargo test --test executor_tests command_chaining::part_080 -- --nocapture` | 152/152 |

The official `coproc.tests` probe must still be treated as bridged: its
Rubash output includes `coproc.right` content from
`execute_upstream_coproc_script`. A direct run showed Bash/Rubash differences
for missing `xcase` and `/etc/passwd` path availability, so those are not
valid semantic closure evidence until the bridge-free body is run. The
upstream bridge remains open and is listed in the separate ledger TODO.

### 2026-08-14 Coproc cat-dash and cross-process termination

The bridge-free official source body was rerun after the coproc follow-up. The
previous timeout was a real semantic failure, not an official-test harness
timeout:

- `cat -` in `coproc REFLECT { cat -; }` bypassed the Rubash streaming cat
  owner and delegated stdin to the host `cat.exe` path. With no file operands,
  `-` now follows the same inherited-stdin streaming path as bare `cat`.
- The background `{ sleep 1; kill $REFLECT_PID; } &` could not terminate a
  blocked child because Windows SIGTERM was queued in the target Rubash
  mailbox. Cross-process Windows signals now use native termination; the
  mailbox remains for signals delivered to the current shell and trap handling.

Focused evidence:

| Command | Result |
|---|---|
| `cargo test --test cli_tests c_command_external_cat_dash_receives_coproc_data_before_writer_close -- --nocapture` | 1/1 |
| `cargo test --test cli_tests c_command_starts_cat_dash_coproc_after_waiting_for_previous_coproc -- --nocapture` | 1/1 |
| `cargo test --test cli_tests coproc -- --nocapture` | 14/14 |

Raw bridge-free comparison is preserved under
`target/issue-suites/results/coproc-actual-20260814-catdash-term/`:

- GNU Bash: rc 0; `REFLECT` status 143; no stderr in this Windows-hosted
  invocation.
- Rubash: rc 1; the coproc output and termination sequence complete. The
  remaining stderr/status differences are missing `xcase`, unavailable
  `/etc/passwd`, and the final closed-pipe diagnostic after fd move/close.

The `coproc` semantic map remains `bridge` until those three items are
classified and the official actual-output row is refreshed. Do not treat the
14/14 focused tests or the `.right` runner as closure of that row.

## 2026-08-14 Ordered Output Migration

The current bounded verification is:

| Command | Result |
|---|---|
| `cargo test --test executor_tests command_chaining::part_080 -- --nocapture` | 149/149 |
| `BASH_RUNNER=D:/Git/bin/bash.exe D:/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-redir` | 1/1 |
| `BASH_RUNNER=D:/Git/bin/bash.exe D:/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-vredir` | 1/1 |

The Rust owner changes are in `src/parser/redirect_assign.rs` and
`src/executor/redirection.rs`. The parser keeps the original `2>&1` ordered
redirect but uses a concrete append node when propagating the already-resolved
stdout path into a compound command. `command_output_fd_state` now starts from
`FdTable` capabilities and only consults legacy fd environment keys for
unmigrated state.

## 2026-08-14 Dynamic FD Move Verification

The active `redir`/`vredir` slice now has a confirmed root cause and a focused
semantic fix. `exec {copy}<&$source` was being treated as an external-command
redirect before the `exec` builtin ran. `command_with_process_substitution_files`
materialized the source virtual fd and advanced its offset to EOF. This made a
following `exec {moved}<&$source-` copy an empty remainder even though Bash
duplicates the source input and only then closes the source.

The fix is in the command execution/fd owner boundary:

- `src/executor/command_execute.rs` keeps dynamic `exec {name}...` on the
  shell-owned path.
- `src/executor/execution_misc.rs` recognizes `&N-` only through the dedicated
  move-aware helper; ordinary redirect classification still accepts only `&N`.
- `src/executor/trap_exec.rs` copies only the input/output side selected by the
  operator, closes move sources, preserves closed dynamic variable values, and
  allows the freed slot to be reused.
- `tests/executor_command_chaining/part_080.rs` covers input-only state and
  input move/close/reuse behavior.

Evidence:

| Command | Result | Raw evidence |
|---|---|---|
| `cargo test --test executor_tests command_chaining::part_080::test_exec_dynamic_ -- --nocapture` | 12/12 passed | Cargo output; generated test files are cleaned by the test |
| `cargo test --test executor_tests command_chaining::part_080 -- --nocapture` | 146/149 passed | Cargo output; remaining 3 are ordered stderr/`<>` failures |
| `BASH_RUNNER=D:/Git/bin/bash.exe D:/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-redir` | 1/1, exit 0 | `target/bash-upstream-tests/logs/run-redir.log` |
| `BASH_RUNNER=D:/Git/bin/bash.exe D:/Git/bin/bash.exe scripts/run-bash-upstream-tests.sh run-vredir` | 1/1, exit 0 | `target/bash-upstream-tests/logs/run-vredir.log` and latest `results.tsv` |

The runner's `target/bash-upstream-tests/results.tsv` is overwritten by the
next focused invocation; the two log files are the durable raw evidence for
the two commands. The broader native probe artifacts remain in
`target/issue-suites/results/native-bash-20260814-vredir/`.

The closed-fd diagnostic probe also established the current Bash-version
boundary: GNU Bash 5.2 prints `read error: 10: Bad file descriptor`, while
Rubash prints `read: 10: invalid file descriptor: Bad file descriptor`. The
Rust regression now asserts the shared Bash error class rather than the old
Rubash-specific `invalid file descriptor specification` text.

Remaining work is explicitly separate: `part_080` still has three failures in
ordered stderr copies and read-write `<>` handling. `vredir4/5/7/8` still need
primitive-level comparison even though the checked-in `.right` runner reports
1/1. No issue is closed by this slice.

## Executive Summary

### 2026-08-14 ambiguous redirect after unquoted expansion

Unquoted parameter expansion in a redirection target now follows Bash's
ambiguous-redirect rule. For example, `target="a b"; echo hi > $target`
previously created a file literally named `a b`; it now reports
`a b: ambiguous redirect`, returns status 1, and does not create the file.
The check runs at the common materialized-command boundary so shell builtins,
external commands, and compound commands share the same behavior. Quoted
targets such as `> "$target"` remain valid.

The same validation now rejects invalid expanded fd operands such as
`fd=-1; exec <&$fd`, reporting `-1: ambiguous redirect` instead of trying to
open `&-1` as a filesystem path. This closes two concrete `redir.tests` fd
diagnostic cases in the shared redirection boundary.

Verification: `cargo test --test cli_tests fd_redirects` (13 passed),
`cargo test --test cli_tests -- --nocapture` (150 passed), and the Bash
`run-redir` slice (1/1).

### 2026-08-13 wait -n completed-status retention

`wait -n` now retains the exit status it reaps so a later explicit `wait PID`
can query that completed child as Bash permits. The status is consumed by the
explicit wait, while ordinary explicit waits still remove the job immediately.
This fixes the jobs/wait root-cause slice tracked by Issues #22 and #23.

Verification: `cargo test --test executor_tests command_chaining::part_036`
(27 passed).

### 2026-08-13 command-substitution heredoc collection

Batch stdin execution now waits for command-substitution syntax to close after
all declared heredoc bodies have been collected. Previously parentheses in a
heredoc body could make the top-level input check report an EOF syntax error
before the body and closing `)` were available. Command-list substitution also
now uses the real parser for heredoc-bearing sources.

Verification: `cargo test --test executor_tests command_chaining::part_005`
(40 passed); Bash upstream `run-heredoc`, `run-comsub`, `run-comsub-eof`,
`run-comsub-posix`, `run-redir`, `run-vredir`, and `run-procsub` (7/7).

### 2026-08-13 shopt -o display mode

`shopt -o option` uses the normal readable `name<tab>on|off` format. Rubash
was incorrectly passing the reusable-print flag unconditionally, producing
`set +/-o name` for the non-`-p` form. The `-p` form remains reusable.

Verification: Bash upstream `run-shopt` (1/1) and `cargo test --lib`
(163 passed).

### 2026-08-13 parameter replacement literal backslashes

Parameter replacement patterns containing an escaped backslash now normalize
the lexer quote markers before matching. This makes `${P//\\\\//}` replace
Windows path separators with `/`, matching Bash, while preserving ordinary
glob pattern semantics.

Verification: `cargo test --test executor_tests command_chaining::part_024`
(13 passed); direct Bash/Rubash probe outputs `C:/work/dir/file.txt`.

### 2026-08-13 Arithmetic Literal Diagnostics Progress

Arithmetic literal failures now preserve the GNU Bash `expr.c::strlong`
diagnostic categories instead of collapsing every invalid `base#digits` form
into `value too great for base`. Rubash now distinguishes invalid bases,
missing constants, out-of-range digits, invalid zero/duplicate-base numbers,
and invalid octal constants. A focused probe for `3425#56`, `2#`, `2#44`,
`0#4`, and `2#110#11` matches GNU Bash output and status.

This is a diagnostic/root-cause slice only. The official `arith.tests` and
`arith-for.tests` artifacts remain red because array/associative-array
semantics, conditional assignment precedence, recursive arithmetic expansion,
and arithmetic-for command-substitution parsing still differ.

### 2026-08-13 Arithmetic Increment Expansion Progress

The focused GNU `arith2.sub` slice now matches Bash completely (stdout, stderr,
and exit status). The root cause was the distinction in GNU `expr.c::exp0` and
`evalexp` between an arithmetic expansion's numeric result and its validity:
`$((--7))`, `$((++ 7))`, and related non-lvalue prefix forms preserve the
operand value in expansion context, while `(( -- ))` and `(( ++ ))` remain
command errors. Rubash previously represented both cases as `Option<i128>`,
dropping the expansion result and producing only 14 of the expected 26 lines.
The evaluator now falls back to the operand for the expansion form and keeps
the command-context diagnostic, including Bash's error-token spacing.

Raw verification artifacts are under
`target/issue-suites/results/arith-focus/current2-final2-rubash.*` alongside
the Bash baseline. The complete arithmetic suites are still not green; array
subscript quote handling and other `arith10.sub` contexts remain separate
work.

### 2026-08-13 Pipeline/Fd Progress

The first Windows pipeline lifecycle slice is now covered by real execution
semantics. Rubash starts eligible plain external pipelines concurrently with
OS pipes, waits for the final consumer, then collects upstream process status
with a short natural-exit window before the Windows hard-kill fallback. This
matches the relevant GNU Bash `execute_cmd.c`/`jobs.c` ordering: close unused
pipe ends, wait for the pipeline job, and publish all member statuses to
`PIPESTATUS` before applying `pipefail`.

The corresponding WinuxCmd `head` implementation reads stdin incrementally,
so an unbounded producer can stop after the requested records. Verified
examples include `yes | head -n 3`, a three-stage external pipeline,
`PIPESTATUS`, `pipefail`, and `|&`. The Windows fallback may report status 1
for a producer that must be force-killed; Windows has no SIGPIPE status 141.
This is an environment-specific status difference, while output, final status,
pipeline completion, and process cleanup are covered by regressions.

Focused verification: `cargo test --lib` 153/153; pipefail executor slice
3/3; fd redirect slice 11/11; CLI pipeline/fd regressions passed. Raw suite
artifacts are not generated by this focused change.

### 2026-08-13 Heredoc Progress

Heredoc delivery now follows Bash's quoted-newline rule: unquoted heredocs
remove an unescaped `\\` plus newline before parameter/command expansion,
while quoted heredocs preserve the body literally. On Windows, eligible plain
external pipelines are spawned completely before the first-stage heredoc is
written. This prevents a large heredoc from filling a pipe before downstream
consumers exist. The concurrent path also permits a final `>`/`>>` redirect;
other pipeline redirects remain on the conservative shell-aware path.

Verified focused behavior includes quoted/unquoted heredocs, `<<-`, command
substitution, an external `cat <<HERE | md5sum`, and BusyBox's
`heredoc_huge.tests`. The latter completed with status 0, matching md5 output,
and `End`, with no stderr. The 120KB pipeline-output regression and heredoc
parser/redirection slices also pass. GNU Bash's large-heredoc tempfile fallback
remains a future optimization; the current fix establishes equivalent
completion semantics for the covered Windows pipeline shape.

The failures are not one-off test noise. They cluster into a small number of
Bash semantic subsystems:

### 2026-08-13 Parser Strictness Progress

The coproc actual-output slice was rechecked against GNU Bash and currently
matches, so no speculative coproc rewrite was made. The next ISSUE-backed
parser gap was `[[ -n & ]]` (and equivalent invalid conditional operators):
Bash reports a syntax error and exits 2, while Rubash previously treated the
unknown expression as an ordinary false condition. The parser now marks
invalid conditional separators and unmatched delimiters for the existing
syntax-error path. Focused parser conditionals and CLI regressions pass;
valid logical conditions and pattern RHS expressions remain accepted.

The follow-up parser slice also now rejects unterminated `[[ ...` commands
with status 2 instead of treating `[[` as an ordinary command. Recovery
consumes input through `]]` when present, or to EOF otherwise, preserving the
parser's ability to continue after a malformed conditional in a script.

Command substitution EOF handling is now covered at the lexer/executor
boundary as well: a genuinely missing `)` in `$(...)` returns status 2 and
does not dispatch the partial body as a command. Closed and nested command
substitutions remain green. A substitution whose outer `)` exists but whose
inner `if`/compound command is malformed is also rejected now: compound
sources are routed through the complete command-list parser, and status 2 is
propagated through the outer command. Closed compound substitutions remain
executable, matching the `subst.c` complete-list path and `parse.y` error
handling.

The case-command slice now rejects an invalid top-level compound terminator
inside a case clause, such as `case x in x) done ;; esac`. Bash treats this as
a syntax error rather than a command in the clause. Case parser coverage
remains green, including nested case bodies and command substitutions.

1. **Redirection / fd / Windows device mapping**: `/dev/null`, `2>&1`, `&1`,
   fd duplication/close, ambiguous redirect diagnostics, coproc fd lifetime.
2. **Heredoc**: delimiter collection, heredoc inside command substitution,
   unterminated heredoc diagnostics, and large heredoc buffering/hang behavior.
3. **Parser strictness**: Bash rejects invalid syntax with rc=2 while rubash
   often accepts and exits 0.
4. **Arithmetic status and diagnostics**: invalid bases, division by zero,
   operand errors, and arithmetic-for syntax do not consistently propagate.
5. **Alias / hash / command lookup**: alias expansion timing, quoted alias
   handling, `alias -p`, tracked aliases, hash-table behavior.
6. **Word expansion**: IFS splitting, quoted arrays, substring/slice, pattern
   substitution, RHS expansion, and command substitution state isolation.
7. **Jobs / coproc / process substitution**: job table semantics, wait/kill
   interactions, fd lifecycle, and timeout/hang behavior.
8. **Builtin parity**: option parsing, output, and exit status for `read`,
   `mapfile`, `shopt`, `trap`, `type`, `printf`, `complete`, `getopts`,
   `umask`, `set`, `cd`, `jobs`, and related builtins.

The direction is therefore clear: fix by root-cause subsystem, not by issue
number or by one-off expected-output patches.

## Architecture Boundary

Rubash is being used as the Bash grammar/execution library for Winuxsh, not as
a thin standalone `bash.exe` clone. The ownership boundary should be:

| Layer | Ownership |
|---|---|
| `rubash` | Bash syntax, AST, expansion, heredoc, redirection semantics, builtin behavior, job/trap semantics |
| `winuxcmd` | Windows-native process control, handle/fd backend, pipe/device primitives, hard-kill primitives |
| `winuxsh` | Host shell, interactive/session/profile wrapper, integration that embeds rubash as a library |

Concrete implication: `&1`, `2>&1`, `/dev/null`, coproc fd handling, and Bash
`kill` semantics should be modeled in rubash, backed by winuxcmd primitives
where Windows process/handle operations are needed. They should not be fixed as
Winuxsh UI behavior.

## Suites Covered

The issue corpus includes more than the few suites rerun in the latest session.
The current compatibility picture comes from these sources:

| Suite | Purpose | Current known result | Artifact / source |
|---|---|---:|---|
| Rust unit/lib | Local implementation regressions | 153/153 pass | `cargo test --lib` |
| Rust kill regressions | Windows kill and shell PID handling | pass after `47924f4` | focused `cli_tests` / `executor_tests` |
| Self difftest cases | Hand-authored Bash vs rubash probes | previously 26-case baseline | `tests/difftest/` |
| Bash upstream `.right` runner | GNU Bash `run-*` against checked-in `.right` expectations | 87/87 historical expectation baseline; separate from actual-output parity | `target/bash-upstream-tests/results.tsv` |
| Bash actual-output | GNU Bash `.tests` bodies, GNU Bash vs Rubash at `b5e3f7b3` with LF-normalized copied fixtures | 83 files: 13 PASS / 70 raw DIFF | `target/issue-suites/results/bash-ledger-refresh-b5e3f7b3/results.tsv` |
| Oil spec | Broad POSIX/Bash/YSH shell behavior corpus | 222 spec files processed; 12 clean, 210 with diffs/failures | `target/issue-suites/results/oils-spec.status` |
| mksh `check.t` | Large shell semantic corpus, Bash-compatible subset used as signal | passed 162; failed 417, including 5 ignored | `target/issue-suites/results/mksh-check-timeout.log` |
| Busybox `ash_test` | POSIX/ash shell behavior, strong heredoc/redir/signal signal | official runner completed; 5 failing scripts in latest run | `target/issue-suites/results/busybox-run-all.log` |
| ksh93 diff | ksh93 regression files, Bash-compatible subset used as signal | 56 files: 25 PASS / 31 DIFF | `target/issue-suites/results/ksh93-diff/results.tsv` |

Notes:

- The Bash upstream `.right` runner is useful but incomplete as a truth source:
  it can pass while Bash 5.2 actual-output still differs.
- ksh93 and mksh include syntax that Bash does not support. Those failures are
  not automatically rubash bugs; only Bash-compatible subsets are repair targets.
- `target/issue-suites` contains temporary runners and logs. It is intentionally
  not the permanent project documentation.

## Bash Actual-Output Result Shape

Latest local rerun:

```text
TOTAL=83 PASS=13 DIFF=70 (current isolated raw ledger; the 15/68 line is historical)
```

Exit-code shape:

| bash_rc/rubash_rc | Count | Interpretation |
|---|---:|---|
| `0/0` | 52 | Both complete; output/diagnostics differ. These are real semantic diffs. |
| `2/0` | 11 | Bash rejects syntax or builtin usage; rubash silently accepts. |
| `127/0` | 12 | Harness/environment or command lookup mismatch; review before classifying. |
| `124/0` | 5 | Timeout/hang or replicated-runner noise; review individually. |
| `1/0` | 2 | rubash drops runtime/arithmetic failure status. |
| `9/0` | 1 | Termination/status mismatch. |

High-signal Bash actual-output families:

| Family | Representative files | Likely implementation area |
|---|---|---|
| Syntax error acceptance | `parser`, `cond`, `errors`, `glob-bracket`, `arith-for`, `array` | `src/parser/*`, conditional/arithmetic/array grammar, parse error propagation |
| Arithmetic diagnostics/status | `arith`, `arith-for`, `cond`, `quotearray`, `new-exp` | `src/executor/arithmetic/*`, `src/executor/arithmetic_aliases.rs` |
| Heredoc / command substitution | `heredoc`, `comsub-eof`, `comsub-posix`, `exportfunc` | `src/lexer/heredoc*.rs`, `src/parser/command_substitution.rs`, `src/executor/command_substitution*.rs` |
| Redirection / fd semantics | `redir`, `vredir`, `coproc`, `procsub`, `read` | `src/executor/redirection.rs`, `src/executor/external_redirects.rs`, `src/executor/builtin_redirects.rs`, `src/sys/sh/zmapfd.rs` |
| Alias/hash semantics | `alias`, `errors`, `history` | `src/builtins/alias.rs`, `src/builtins/hash.rs`, `src/executor/alias_*.rs` |
| Word splitting / quoting / arrays | `ifs`, `nquote*`, `quotearray`, `rhs-exp`, `assoc`, `array` | `src/executor/expand_word.rs`, `src/executor/parameter_words.rs`, `src/shell/arrays/*` |
| Glob/extglob/brace | `glob`, `globstar`, `extglob`, `braces` | `src/executor/glob.rs`, `src/parser/extglob_pattern.rs`, `src/expand/braces.rs` |
| Builtin option coverage | `complete`, `getopts`, `mapfile`, `read`, `shopt`, `trap`, `type`, `test`, `printf` | `src/builtins/*` |
| Jobs/coproc/process substitution | `jobs`, `coproc`, `procsub`, `varenv` | `src/jobs/*`, `src/parser/coproc_command.rs`, `src/parser/process_substitution.rs`, executor pipe/fd handling |

Concrete redirection/device evidence:

- `vredir6.sub`: `/dev/null: Invalid argument`
- `redir.tests` / `vredir*.sub`: `$fd: Bad file descriptor`,
  `$fd: ambiguous redirect`, `-1: ambiguous redirect`
- `coproc.tests`: coproc descriptors trigger `Bad file descriptor`

These point to an incomplete fd/device model, not just missing output strings.

## ksh93 Diff Result Shape

Latest local rerun:

```text
TOTAL=56 PASS=25 DIFF=31
```

High-signal files where rubash has sharply more failures than Bash:

| File | Bash errors | Rubash errors | Likely implementation area |
|---|---:|---:|---|
| `arith.sh` | 2 | 127 | arithmetic parser/evaluator; ignore ksh-only enum noise |
| `attributes.sh` | 2 | 125 | `typeset`/`declare` attributes and assignment attributes |
| `case.sh` | 7 | 40 | case pattern matching and parser/runtime dispatch |
| `heredoc.sh` | 24 | 127 | heredoc parser/lexer/runtime boundary |
| `jobs.sh` | 2 | 10 | jobs/wait/kill job-control semantics |
| `path.sh` | 1 | 103 | path search, command lookup, script execution paths |
| `quoting.sh` | 51 | 125 | quoting and word expansion |
| `quoting2.sh` | 2 | 65 | quoting and word expansion |
| `statics.sh` | 2 | 127 | shell variable state, readonly, scope semantics |
| `subshell.sh` | 2 | 127 | subshell state isolation |
| `substring.sh` | 2 | 127 | parameter expansion substring/slice |
| `timetype.sh` | 2 | 16 | `time` and typed-variable semantics |
| `vartree1.sh` | 2 | 5 | variable-tree and compound-variable noise |

Classification rule:

- ksh-only constructs such as `enum`, compound variables, `.sh.*`, and typed
  attributes are not Bash compatibility requirements.
- Bash-compatible constructs inside these files still matter, especially
  heredoc, quoting, case, arithmetic, path lookup, jobs, and substring behavior.

## Busybox / mksh / Oil Signals

These suites reinforce the same root-cause groups:

| Suite | Strongest signal | Root-cause family |
|---|---|---|
| Busybox ash | heredoc_huge, redir, signal, vars, parsing | heredoc, fd/device, parser strictness, signal/job semantics |
| mksh | heredoc, integer-base, alias, arrays, command substitution, variable expansion | heredoc, arithmetic, alias, arrays, word expansion |
| Oil spec | word-split, trap/umask/kill/set/cd/echo, redirects, var-op, parse-errors, command-sub | word expansion, builtin parity, redirection, parser strictness, command substitution |

The suites disagree in surface syntax but agree on implementation gaps.

## What Is Missing vs. What Is Merely Incomplete

| Area | Status | Direction |
|---|---|---|
| fd/device model | Partially implemented, incomplete on Windows devices and fd duplication | Build a central Bash fd abstraction backed by winuxcmd handles; normalize `/dev/null`; implement dup/close/error semantics before patching individual tests |
| heredoc | Implemented but semantically incomplete and performance-risky | Fix lexer collection and runtime delivery; add large heredoc regression and command-substitution heredoc cases |
| parser errors | Implemented permissively in places | Add strict Bash parse errors and rc=2 propagation for invalid conditionals, arrays, extglob, arithmetic-for, and substitutions |
| arithmetic | Implemented but error propagation incomplete | Make invalid constants/base/division/syntax produce Bash-compatible diagnostics and statuses |
| alias/hash | Implemented but missing timing/listing/tracked-alias behavior | Align alias expansion timing and builtin output/status; decide whether tracked alias is needed or explicitly scoped |
| word expansion | Broadly implemented but many edge cases remain | Fix IFS empty fields, quoted arrays, substring/slice, patsub, RHS and command substitution state isolation |
| jobs/coproc/process substitution | Partially implemented; fd lifecycle incomplete | Define job table and coproc fd ownership; connect wait/kill/signal behavior to winuxcmd backend |
| builtins | Many exist but option/error parity is uneven | Finish builtin-by-builtin parity using GNU Bash `.def` behavior and issue suite examples |

## Repair Order

1. **fd/device/redirection**: `/dev/null`, `2>&1`, `&1`, fd duplicate/close,
   ambiguous redirect, bad fd diagnostics.
2. **heredoc**: ordinary, command-substitution, unterminated, and huge heredoc.
3. **parser strictness**: rc=2 cases that rubash currently accepts.
4. **arithmetic error propagation**: invalid base, division by zero, syntax.
5. **jobs/coproc/process substitution**: fd lifecycle and wait/kill behavior.
6. **word expansion**: IFS, quoting, arrays, substring/slice, patsub.
7. **builtin parity**: trap/umask/kill/set/shopt/read/mapfile/type/printf/etc.
8. **alias/hash**: expansion timing, listing, tracked alias/hash table behavior.

This order prioritizes correctness foundations that many suites share. Fixes
should be validated with focused Rust tests first, then the smallest relevant
suite slice, then the broader issue-suite family.

### 2026-08-13 Arithmetic Checkpoint

Compared with GNU Bash `execute_cmd.c`/`expr.c`, arithmetic-for now preserves
the failure status for invalid initialization, test, and update expressions.
The previous `!ran_body` cleanup path could overwrite initialization/test
errors with status 0. The executor now tracks arithmetic failure separately,
reports the arithmetic diagnostic, and returns status 1 while retaining the
normal zero-iteration status for a valid false test.

Focused validation:

```text
cargo test --test cli_tests arithmetic -- --nocapture         PASS (3/3)
cargo test --test parser_tests arithmetic_for -- --nocapture PASS (4/4)
cargo check                                                    PASS
```

Remaining arithmetic work is broader expression parity: complete invalid
lvalue diagnostics, `let` compound-expression parsing/status, conditional
arithmetic errors, and the Bash actual-output `arith`/`arith-for` suite slice.

### 2026-08-13 Arithmetic Expansion Follow-up

GNU Bash `expr.c::evalexp` reports an invalid expression through `validp`,
while `subst.c` distinguishes a failed arithmetic expansion in an ordinary
word from a failed assignment expansion. Rubash now follows that boundary:

- lexer continuation scanning treats `$((...))` as arithmetic context, so `#`
  in a base literal is not mistaken for a shell comment;
- ordinary words such as `echo $((2#44)); echo after` fail only the current
  command and allow the following command to run;
- assignment words still abort the current command list on arithmetic failure;
- `let`, `(( ))`, `[[ ]]`, arithmetic-for, and embedded arithmetic expansion
  paths now report a diagnostic even when the exact parser token is unknown.

Focused results are stored under
`target/issue-suites/results/arith-status-*` and
`target/issue-suites/results/arith-base-*`. Rust arithmetic tests remain green
(5/5); the official `arith.tests` and `arith-for.tests` files still have
broader unrelated differences and are not yet classified as passing.

### 2026-08-13 Arithmetic Empty/Subscript Progress

Compared with GNU Bash `expr.c::subexpr` and
`arrayfunc.c::array_value_internal`, Rubash now treats empty arithmetic
expressions as zero and performs quote removal inside indexed arithmetic
subscripts before evaluating the index, including ordinary indexed-array
assignment words such as `a[" "]=10` and `a[""]=23`. `$(( ))`, `$((\"\"))`,
`[[ 0 -eq \"\" ]]`, and `(( a[\" \"]=11 ))` match Bash in focused probes.
The empty-expression and indexed-subscript behaviors are covered by CLI
regressions.

The remaining `arith10.sub` mismatch is narrower: assignment words using
escaped quotes such as `a[\\\" \"]=15` are still split incorrectly by the
lexer/parser before the array-assignment owner receives them. No broad lexer
heuristic was retained after it failed to improve the exact reproducer.

## 2026-08-12 Repair Checkpoint

### 2026-08-13 umask symbolic permission progress

The next builtin-family gap was in `src/builtins/umask.rs`. Compared with
GNU `builtins/umask.def::parse_symbolic_mode`, Rubash accepted only `rwx`
permissions in symbolic modes. Bash also accepts permission-copy operands
(`u`, `g`, `o`) and conditional execute permission (`X`), which are used by
the upstream `builtins8.sub` coverage.

Rubash now maps copied permissions from the selected source class and applies
`X` only when the current allowed mode has an execute bit, while retaining the
shell-local umask state used by the Windows host. Added unit coverage for
`o=u`, `o=g`, and `a+X`.

Verification:

```text
cargo test --lib umask: 3 passed
cargo test --test cli_tests umask: 1 passed
run-builtins: 1/1 passed
```

The follow-up comparison with `umask.def::parse_symbolic_mode` also fixed two
details that the first implementation missed. Permission-copy operands and
`X` now use the mode bits from before the complete clause list, matching
Bash's `initial_bits` argument even after an earlier clause modifies the
working mask. Invalid numeric modes and invalid symbolic modes now preserve
Bash's diagnostic categories (`octal number out of range`, invalid symbolic
operator, or invalid symbolic character) instead of collapsing them into one
message. Five unit tests cover the successful and diagnostic paths.

### 2026-08-13 trap action-only inspection

GNU `builtins/trap.def::trap_builtin` supports `-P` in addition to `-l` and
`-p`. `-P` prints only the stored action for the specified signal, requires at
least one signal name, and cannot be combined with `-p`. Rubash previously
treated `-P` as an invalid signal specification because its option handling
only recognized `-l`.

`src/builtins/trap.rs` now parses the three GNU display options, preserves the
existing reusable `trap -- ACTION SIGNAL` output for `-p`, and emits action-only
output for `-P`. Invalid options and the two `-P` usage errors return status 2.
This is shell-table behavior; native signal delivery remains owned by the
executor/backend boundary.

Verification:

```text
trap-related executor tests: 22 passed
cargo test --test cli_tests trap: 2 passed
run-trap: 1/1 passed
cargo check: passed
```

### 2026-08-13 shopt reusable-query status

GNU `builtins/shopt.def::list_shopts` and `list_shopt_o_options` return failure
when a specified option is disabled, even when `-p` prints its reusable form.
Rubash printed the correct `shopt -u NAME` or `set +o NAME` line but returned
success from those `-p` paths. The executor now preserves the Bash query status
