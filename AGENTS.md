# Rubash Agent Entry Point

Before making compatibility changes, read:

1. `docs/gnu-bash-compatibility-implementation-plan.md`
2. `docs/issue-suite-diff-analysis.md`
3. `docs/bash-compat-issues.md`
4. `docs/bash-source-map.md`

Key rules:

- **CRITICAL: Always use WSL GNU Bash (`wsl bash`, **5.3.0** at /usr/local/bin/bash — owner-compiled, baseline directive 2026-09-09; legacy 5.2.21 reference via scripts/true-baseline-521.sh) for semantic comparisons — NOT the winuxsh shim, and NOT Git Bash.** The winuxsh shim at PATH `bash` is an older version with different behavior. Git Bash (`D:/Git/bin/bash.exe`) is below rubash in some areas (notably quoting/escaping/braces) and produces wrong baselines there. Compare with a script FILE passed to both shells (see Bash Test Suite below).
- **GNU C source is the specification.** Every semantic change must cite the
  owning C function (`file:line func`) in `third_party/bash/` before editing.
  Do NOT derive behavior from a suite's diff count, from a local pass/fail
  matrix, from memory, or from `/usr/bin/bash` (5.2.21) / Git Bash / the
  winuxsh shim. If the C source does not settle the question, say so and mark
  the change unverified instead of guessing. See *GNU C source is the
  specification* below.
- Fix by root-cause subsystem, not by individual expected-output lines.
- **No whack-a-mole guards.** Do not extend blacklist predicates
  (`contains`/`starts_with` admission lists) on word-level fast paths with
  another symptom check; fix the wrong invariant or flip admission to a
  whitelist that falls through to the real parser. See *No whack-a-mole
  guards* below (rubash#117).
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

### GNU C source is the specification — never guess

Rubash's contract is "behave like GNU bash 5.3.0". That contract is defined by
the C source in `third_party/bash/`, so **the C source is the specification** —
not the test output, not a diff count, and not what "looks symmetric".

Before changing any semantic behavior:

1. **Cite it.** Find the C function that owns the behavior and record
   `file:line function_name` in the task report and the commit message.
   Example: `parse.y:5366 read_token_word()` backslash / `PST_NOEXPAND` branch.
2. **Probe it.** Run the minimal case through WSL GNU Bash 5.3.0 *from a script
   file* (see Bash Test Suite) and paste the raw output.
3. **Then edit**, naming the Rust semantic owner and the changed invariant.

Forbidden substitutes for reading the source — each has already produced a wrong
"fix" in this repo:

- Inferring semantics from a suite's diff count, or from a hand-built pass/fail
  matrix. A green matrix only proves your model matches the cases you happened
  to test.
- Building the rule from "correct-looking symmetry" between parallel code paths
  (comsub vs non-comsub, quoted vs unquoted, one builtin vs another).
- Trusting remembered bash behavior, or behavior observed under `/usr/bin/bash`
  (5.2.21), Git Bash, or the winuxsh shim. None of those is the oracle, and they
  differ exactly where rubash needs the truth.
- Generalizing from one example character to a whole class without reading the
  character-class tables in the C code (`sh_syntaxtab`, `CBSDQUOTE`,
  `shellbreak()`, `sh_shellmeta()`).

If the C source does not settle the question, say so explicitly and mark the
change unverified. Do not invent a rationale.

**Worked counter-example (2026-09-13 — keep this one).** A `\$`-inside-`$()`
quote-removal fix was designed from a 19/20 pass matrix: it did quote removal at
comsub-body split time and simulated `CTLESC` with carrier bytes. Reading the
source afterwards showed the model was structurally wrong — GNU does **not**
strip the backslash inside a command substitution. `parse_comsub()`
(`parse.y:4451`) sets `PST_NOEXPAND` (`parse.y:4513`), defined at `parser.h:51`
as *"don't expand anything in read_token_word; for command substitution"*. The
backslash branch of `read_token_word()` (`parse.y:5368-5375`) responds by
*keeping* the `\` in the token, setting `quoted = 1`, and using
`pass_next_character` to mark only the following character
(`parse.y:5358-5362`) — quote removal is deferred to the inner execution, not
performed at split time. The matrix-driven version passed 19/20 cases and still
introduced two new divergences (`$(echo \*)` globbed the cwd; `$(echo "\\")`
printed `"` instead of `\`). A green local matrix is not evidence.

### Verified GNU entry points

Line numbers are for the vendored tree (`bash.git @b4608166`, Bash-5.3 patch 15).
Re-verify with `grep -n` before citing; treat this table as a starting point, not
gospel.

| Topic | Authoritative location |
| --- | --- |
| Word/token assembly, quoting state | `parse.y:5305 read_token_word()` |
| Backslash handling + `PST_NOEXPAND` | `parse.y:5366-5398`, `pass_next_character` at `5358-5362` |
| Where `CTLESC`/`CTLNUL` get protected | `parse.y:5694-5706` (`got_character` vs `got_escaped_character`) |
| `PST_NOEXPAND` meaning and scope | `parser.h:51`; cleared at `parse.y:7048`, `parse.y:7127` |
| `${...}` text matching | `parse.y:3877 parse_matched_pair()` |
| Command substitution body parse / reprint | `parse.y:4451 parse_comsub()`, `parse.y:4632 print_comsub()` |
| Word expansion driver | `subst.c:11229 expand_word_internal()` |
| `${parameter...}` expansion | `subst.c:9777 parameter_brace_expand()`, `subst.c:7663 parameter_brace_expand_word()` |
| Command substitution execution | `subst.c:7143 command_substitute()` |
| Verbatim extraction | `subst.c:1148 string_extract_verbatim()` |
| `CTLESC` dequote only | `subst.c:4692 dequote_escapes()`, `subst.c:4901 remove_quoted_escapes()` |
| Final dequote | `subst.c:4807 dequote_string()`, `subst.c:4865 dequote_word()` |
| Expanding a word list | `subst.c:13219 expand_word_list_internal()` |
| Redirection word expansion | `redir.c:298 redirection_expand()` |
| Command execution dispatch | `execute_cmd.c:624 execute_command_internal()` |
| Job wait | `jobs.c:3064 wait_for()`, `nojobs.c:795 wait_for()` |
| Variables and attributes | `variables.c`; builtin option tables under `builtins/*.def` |

### Carrier bytes are a `CTLESC`/`CTLNUL` port, not a local trick

The in-band carrier bytes rubash uses to encode quoting state stand in for GNU's
`CTLESC` / `CTLNUL` mechanism. This is the highest-risk area of the codebase.
Any change touching them must be justified against `parse.y:5694-5706`,
`subst.c:4692 dequote_escapes()`, `subst.c:4807 dequote_string()` and
`subst.c:1148 string_extract_verbatim()`, and validated with a full 83-suite
ledger. A focused suite count is not enough: `$'...'`, PUA markers and carrier
handling have regressed under edits that looked obviously safe.

### No whack-a-mole guards on text-layer fast paths — converge to the real parser (rubash#117)

This is a hard rule, established by rubash#117 after the quote/word-splitting
bug family passed half of all engine bugs (#59/#60/#64/#68/#69/#70/#81/#82/
#91/#95/#96/#97/#109/#116, niubash#119). Every one of those was produced by a
handwritten word-level fast path that re-implements lexer semantics in text,
and every previous fix added one more blacklist condition that missed the next
case.

**Forbidden:** "found a leak → add one more guard" fixes. If a shortcut's
admission is decided by a growing list of `contains("X")` / `starts_with("Y")`
conditions, do not add a sixth. Two sanctioned moves only:

1. **Fix a wrong invariant.** When an existing predicate's contract is simply
   false (example: `command_substitution_quotes_are_semantic` claimed
   "top-level quotes only group words", but a quote wrapping `$`/`` ` ``/glob
   also suppresses field splitting and pathname expansion), correct the
   predicate so it covers the class — one check for the whole class, not one
   per symptom.
2. **Invert admission to a whitelist, or delete the shortcut.** The fast path
   may run only when the body is provably trivial (pure literal, no quoting,
   no expansion, no operators). Everything else goes to the real
   parser/executor. A false positive in a whitelist only costs speed; a false
   negative in a blacklist is a silent semantic bug.

**Justification is GNU C source, not symmetry.** Before editing any
`split_shell_words*`/`expand_aliases`-fed shortcut, open the owning GNU
function (`subst.c:7143 command_substitute()` → `parse_and_execute`,
`parse.y:4451 parse_comsub`, `subst.c:11229 expand_word_internal`) and state in
the commit/PR: what GNU does here, why the shortcut is or is not equivalent.
GNU has *no* word-level substitution shortcuts at all — the only sanctioned
short-circuit is `parse_string_to_command`'s empty-command check. If a rubash
shortcut exists anyway, its comment must name which GNU behavior it preserves
and which inputs disqualify it.

**Verification bar:** a diff matrix against WSL GNU Bash 5.3.0 covering the
whole class — assignment RHS / argument position × literal / variable /
substitution / arithmetic × with-and-without spaces — not just the new
reproducer. A green 19/20 matrix has already shipped wrong "fixes" here
(see the worked counter-example above); match GNU byte-for-byte on the class
or mark the change unverified.

### Keep the measurement honest

- **Build before you measure.** A ledger from a stale binary is worse than no
  ledger. Confirm the script's build step actually succeeded — a failed
  `cargo.exe` lookup leaves the old binary in place and the script still prints
  numbers.
- **A clean environment is part of the measurement.** Before any baseline run on
  the Windows/niu side: `unset BASH_ENV; unset -f rm rmdir unlink; unset
  WINUXSH_ROOT`, then start WSL. Diagnostic: if a quoting-sensitive suite moves
  by a large positive amount (e.g. `dstack 0 -> 50`), `WINUXSH_ROOT` leaked and
  the whole run is void.
- Report environment-bound suites separately from rubash-caused ones, and state
  which project owns each line (see the WinuxCmd section below).

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

- rustfmt is a pre-commit hook (owner directive 2026-09-26): `git config
  core.hooksPath scripts/git-hooks` is already set repo-side; the hook runs
  `cargo fmt --check` and rejects unformatted commits. Run `cargo fmt` before
  committing; do not bypass the hook with --no-verify.

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

