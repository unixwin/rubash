# Rubash Agent Entry Point

Before making compatibility changes, read:

1. `docs/gnu-bash-compatibility-implementation-plan.md`
2. `docs/issue-suite-diff-analysis.md`
3. `docs/bash-compat-issues.md`
4. `docs/bash-source-map.md`

Key rules:

- **CRITICAL: Always use WSL GNU Bash (`wsl bash`, **5.3.0** at /usr/local/bin/bash — owner-compiled, baseline directive 2026-09-09; legacy 5.2.21 reference via scripts/true-baseline-521.sh) for semantic comparisons — NOT the winuxsh shim, and NOT Git Bash.** The winuxsh shim at PATH `bash` is an older version with different behavior. Git Bash (`D:/Git/bin/bash.exe`) is below rubash in some areas (notably quoting/escaping/braces) and produces wrong baselines there. Compare with a script FILE passed to both shells (see Bash Test Suite below).
- Fix by root-cause subsystem, not by individual expected-output lines.
- Keep raw suite artifacts under `target/issue-suites/results/`; keep durable
  interpretation in `docs/`.
- Do not run full suites unbounded. Use per-test, per-file, or per-directory
  timeouts where possible.
- Do not remove `src/executor/upstream_scripts*` until the corresponding
  behavior is covered by real semantics and suite slices are green.
- Treat `src/builtins/kill.rs` and `src/input/readline/kill.rs` as different
  semantic owners: Bash builtin vs readline editing.
- Check for stuck `rubash.exe` / `bash.exe` / suite runner processes before
  finishing a testing turn.

## GNU Source and LLDB Debugging

Compatibility changes must begin from the corresponding GNU Bash C source and
upstream test body. Record the GNU source function/line range, the Rust semantic
owner, and the observable probe before editing. For parameter expansion use
`third_party/bash/subst.c` and `parse.y`; for redirection/fd behavior use
`redir.c`; for execution state use `execute_cmd.c` and `variables.c`.

Use LLDB for Rust runtime control-flow and state inspection when a focused
mismatch is not explained by source reading alone. This repository uses the
MSVC Rust toolchain, so `rust-lldb` is not applicable; invoke the native LLVM
LLDB binary directly (currently `lldb.exe`) against `target/debug/rubash.exe`.
Prefer a script-file target and a noninteractive command file, for example:

```text
settings set target.inline-breakpoint-strategy always
breakpoint set --name 'rubash::executor::...'
run target/probe.sh
thread backtrace all
frame variable
quit
```

Use symbol lookup (`image lookup -n`, `breakpoint list`) before relying on
source line breakpoints, because optimized/incremental Windows builds may move
or omit lines. Capture LLDB stdout/stderr under
`target/issue-suites/results/`; remove temporary instrumentation before
finishing. LLDB evidence complements, but does not replace, the required WSL
GNU Bash 5.3.0 script-file comparison. Do not claim a fix from an LLDB-only
run.

Do not repeatedly rebuild entire suites while locating a root cause. First use
LLDB on the smallest reproducer, then run the focused Rust regression, then run
`run-83.sh check NAME` with bounded timeouts.

## External bashdb

`bashdb` is available as an external Bash-script debugger for Rubash
compatibility work. Use it to debug shell-script behavior running under
`target/debug/rubash.exe`: source mapping, stepping, function stacks, traps,
`eval`, arrays, options, and other Bash semantics. Do not treat bashdb as a
Rust-source debugger; use Rust tooling, logs, instrumentation, and focused tests
for `src/**/*.rs` internals.

The verified local fixture is `target/bashdb-clean/bashdb-generated` with its
library directory at `target/bashdb-clean`. Keep bashdb external and clean: do
not patch bashdb as the product fix. Temporary instrumentation in
`target/bashdb-clean` is allowed for diagnosis only, and must be reverted before
finishing.

Quick smoke test:

```sh
export TERM=xterm DARK_BG=0
printf 'list\nstep\nnext\nwhere\ncontinue\nwhere\nquit\n' | \
  target/debug/rubash.exe target/bashdb-clean/bashdb-generated --no-highlight target/bashdb-probe-target.sh
```

A passing smoke test exits `0`, has empty stderr, lists the target script, steps
into `foo`, prints a stack with `where`, and continues through `42` / `done`.
For setup details, launcher/libdir terminology, and fresh-checkout usage, see
`docs/bashdb-debugging-rubash.md`. Treat full bashdb command coverage as a
development target: each failing bashdb command should normally drive a Rubash
root-cause compatibility fix, not a bashdb patch.

## Bash Test Suite

Compatibility status is tracked in `docs/COMPATIBILITY-STATUS.md` — the single
authoritative source; update it only after real reproduction.

- **Upstream test files**: `third_party/bash/tests/<name>.tests`, run per-file
  with bounded timeouts; keep raw artifacts under
  `target/issue-suites/results/`.
- **Comparison baseline**: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`; legacy 5.2.21 via `scripts/true-baseline-521.sh`). Run the same case file through
  both shells, e.g. `target/debug/rubash.exe case.sh` vs
  `MSYS_NO_PATHCONV=1 wsl bash /mnt/d/repo/rubash/case.sh`.
- **Never use `wsl bash -c "$c"` for cases with doubled backslashes or
  multi-level quoting**: the wsl.exe command-line passthrough collapses
  `\\` to `\`, corrupting the baseline.
- **`scripts/run-83-tests.sh` is retired**: currently broken (`set -u`
  arithmetic + path errors); its historical `17/83` ledger must not be used
  for judgment.

### THIS_SH in GNU Test Scripts

GNU test scripts (e.g. `arith-for.tests`) use `${THIS_SH}` to invoke the
shell under test recursively (`` `${THIS_SH} -c '...'` ``). Rubash now
auto-detects its own executable path via `std::env::current_exe()` and
sets `THIS_SH` in its internal env during `Executor::new()`. This means
`${THIS_SH}` works out-of-the-file without any parent environment
inheritance.

**WSL interop caveat**: The `env` command (including winuxsh's builtin)
does NOT forward env vars from a Linux parent to a Windows child process.
Never use `env THIS_SH=... rubash.exe`. If you must set `THIS_SH`
externally (e.g. in a shell script), use `export`:
```sh
export THIS_SH=/path/to/rubash.exe && rubash.exe script.tests
```
Or rely on rubash's auto-detection (preferred).

### STDERR Output Ordering

GNU bash flushes stderr immediately per write; rubash inherits the default
Windows line-buffered stderr. When a test mixes stderr diagnostics with
stdout output (e.g. arithmetic errors interleaved with loop output), the
ordering may differ between rubash and GNU. This is a known limitation
tracked in arith-for and other suites; fix it by flushing stderr after
diagnostic writes in `eprintln!` paths if ordering matters for a specific
test.

## Multi-Agent / Handoff Discipline

When work is parallelized across agents (AgentTeams or subagents) or handed off
between sessions, the shared working tree is the only durable memory. These
rules prevent the failure modes observed during the 83-test GNU-compat push:
untracked half-finished edits that break the whole build, CRLF that silently
kills WSL-side test runs, and inflated PASS claims from wrong baselines.

- CRLF is a build/test breaker, not cosmetics. The repo has core.autocrlf=true;
  without explicit attributes Git re-CRLFs every .sh/.rs on checkout. WSL bash
  then chokes on spurious CR (dollar-single-quote CR: command not found).
  .gitattributes pins *.sh eol=lf and *.rs eol=lf for this reason. If a test
  harness suddenly reports that error, do NOT patch the script; run
  git add --renormalize . and re-run. The attribute is the only durable fix.

- One verification baseline only: WSL GNU Bash 5.3.0 via scripts/true-baseline.sh (suite slices: `MSYS_NO_PATHCONV=1 wsl bash /mnt/d/repo/rubash/scripts/true-baseline.sh NAME`; the older run-83.sh check remains usable for non-baseline spot checks).
  Do NOT certify a fix by diffing third_party/bash/tests/*.tests against recho/
  zecho output, by comparing raw stdout, or by any other harness. A PASS claim
  is only valid if run-83.sh check prints PASS NAME.

- src/lexer/continuation.rs is captain-exclusive. It carries the family C/E
  quote-leak fix (has_unclosed_quotes skipping dollar-brace, dollar-paren,
  backtick, and dollar-single-quote as self-contained units). Members must NOT
  edit it; if a task needs a lexer change there, send the proposed diff to the
  captain and let them apply it. This avoids clobbering the committed fix.

- Shared-tree hygiene: every edit must cargo build clean before you leave it in
  the tree. Never leave a half-finished function (e.g. a free fn outside its
  impl, or a stray eprintln DEBUG) because it blocks the whole crew's build.
  Do not git stash/pop other agents work to get around a build break; report
  the break instead.

- Commit discipline: the captain groups and commits per-author, per-family
  changes after WSL-baseline verification. Agents do NOT self-commit; they
  report a list of files, the owning task, and the verification result, then
  wait. Do not stack unverified edits across many files.

- Honest handoff: report real diff line counts from run-83.sh check, not
  aspirational ones. If a task is a deep subsystem (e.g. the typed-carrier
  Vec<u8> word carrier for NUL/C0 bytes, or background-job thread
  internalization) and cannot be finished in one pass, say so and stop; do not
  ship a partial change that flips a couple of lines while claiming the family
  is done.


## WinuxCmd / Winuxsh Tooling Issues

WinuxCmd (`D:/repo/unixwin-winuxcmd`, C++ toolchain providing GNU-compatible
external commands: `sed`, `cp`, `grep`, `awk`, `ls`, ...) and Winuxsh
(`D:/repo/unixwin-winuxsh`, the Rust shell shim that is `$0` in these sessions)
are **separate projects from rubash**. A suite diff whose cause is one of their
binaries is not a rubash bug and must not be fixed by patching rubash.

**Required handling when a diff is traced to a WinuxCmd binary:**

1. File an issue at `https://github.com/unixwin/WinuxCmd/issues` with
   reproduction steps, expected vs actual output, and the exact binary/version
   (e.g. WinuxCmd 1.0.3).
2. Apply the repository's real labels (check `gh label list --repo
   unixwin/WinuxCmd` — do not invent new ones):
   - `compat-gap` — 与 GNU 行为不一致，已复现待修. Use when the difference is
     reproduced byte-for-byte and is fixable on Windows (e.g. `sed` brace
     syntax). Pair with `bug`.
   - `compat-candidate` — 疑似兼容差距，待差分验证. Use only while the
     difference is suspected but not yet byte-verified.
   - `noise-platform` — 平台特定噪声. Use when the difference cannot be
     fixed without Linux interop, so it is noise rather than a gap.
   - `wording` / `compat-wording` — same semantics, different message text.
3. Link back to the rubash artifacts in the issue body so the fix can be
   verified: `target/issue-suites/results/true-baseline/<suite>/diff.txt`
   (raw artifacts) and the classifying probe.
4. **Do not count these lines in the rubash compatibility ledger.** Report
   suite numbers split into rubash-caused vs WinuxCmd-caused, and say which
   issue number covers the environment-bound lines.

**Verified examples (WSL GNU Bash 5.3.0 as sole baseline, single
`scripts/true-baseline.sh` harness):**

- `posixexp`: 169 diff lines — 145 from WinuxCmd `sed` rejecting brace `{ }
  scripts (`trim_od()` in `posixexp5.sub`), 1 from WinuxCmd `cp` printing the
  source path on success. **23 lines are true rubash gaps.**
- `globstar`: 101 diff lines — **root cause ESTABLISHED, it is WinuxCmd
  `ls`, not rubash.** `globstar.tests` line 41 is `ls lib/**`. In a rebuilt
  fixture both shells produce a byte-identical 17-word expansion
  (`echo lib/**` → same list, same order, same 4 directories), so rubash
  globstar is correct. The difference is `ls` argument grouping: GNU
  coreutils 9.4 sorts ALL non-directory args before directory args
  (`compare_qsort` uses S_ISDIR as the primary key) and prints the `dir:`
  header blocks after, while WinuxCmd `ls.exe` 1.0.3 groups each directory
  with its own contents inline — which both drops the flat up-front file
  list and duplicates every file (once relative inside its dir block, once
  as the full glob path). A lane agent first called this "dirs-first
  ordering", which is wrong: plain `ls` sort order is identical on both.
  Fix target: `compare_qsort` files-before-dirs grouping.

**Do not use the niubash/winuxsh tool shell for measurement.** It mangles
glob expansion, `printf` glob args, inline `awk`/`sed` one-liners, and
command-substitution pipelines containing `od`/`tr`/`wc`. Write the probe to
an LF-terminated `.sh` file and run it with `wsl bash <file>` on the GNU side.

## Compatibility Push Handoff (2026-09-02)

- Current high-value globstar evidence: `target/multi-gnu.out`,
  `target/multi-rub.out`, and `target/multi-diff.txt`. Preserve verified
  adjacent-`**` collapse, trailing-slash, and single-`**` fast paths while
  investigating multiplicity. Only `run-83.sh check globstar` can close the family.
- Current casemod follow-up is the bare associative route (`AA1^^`,
  `([FOO]=BAR)`). Start from GNU `subst.c` and a minimal WSL script; do not
  infer a Rust bug from environment-sized `declare -p` output.
- Invocation/lexer changes must be reviewed at call sites and with focused tests;
  a clean compile alone is not semantic evidence. Keep `src/lexer/continuation.rs`
  captain-exclusive.
- Shared-tree agents must report exact files, ownership, before/after counts,
  raw artifact paths, and the authoritative command used. The captain stages only
  reviewed files, verifies `git diff --cached`, commits with `git commit -F`,
  checks `git log`, and treats push transport failure as unresolved.
- Remove accidental coordination artifacts and scratch files before handoff; do
  not stage `.agent-teams/`, `hd-out.txt`, or `x` unless explicitly required.

