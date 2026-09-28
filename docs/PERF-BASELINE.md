# Perf Baseline (perf-suite lane, wt4/perfsuite)

First full run of the dedicated performance suite: 24 numbered probes under
`benchmarks/`, executed by `scripts/run-perf-suite.sh` against BOTH
`target/debug/rubash.exe` and WSL GNU Bash 5.3.0. This file is the recorded
baseline; re-run the suite after perf work and diff against this table.

## How to run

```sh
cargo build                     # build BEFORE measuring (AGENTS.md rule)
bash scripts/run-perf-suite.sh  # full suite, both sides, ~8 min
```

Per-probe metadata (`# PERF: runs=N timeout=S args="..." file=... nofile=1`)
is documented in the harness header. TSVs land in
`target/perf-results/{rubash,gnu}.tsv`, the merged table in
`target/perf-results/PERF-TABLE.md`.

## Methodology

- **hyperfine is not used.** It is not installed on this host, and per-run
  `wsl.exe` round-trips would corrupt the GNU side. A portable harness is used
  for BOTH sides (methodology parity): 1 validation run (rc gate, warmup) +
  N timed runs; wall clock via `date +%s%N`; median reported; every run
  wrapped in `timeout -k 5` (SIGKILL follow-up because interactive shells
  ignore SIGTERM).
- The GNU side is timed **inside WSL** by re-invoking the same script with
  `--inner`, so `wsl.exe` startup cost never lands in the numbers. The inner
  pass sanitizes `PATH=/usr/local/bin:/usr/bin:/bin` (WSL interop otherwise
  appends Windows dirs and poisons PATH-lookup timings: probe 10 measured
  16.9 s/run unsanitized vs 24 ms sanitized) and pins `HOME=/tmp/...`.
- Both TSVs are resumable; the outer harness retries an incomplete GNU pass
  (long no-output `wsl.exe` sessions were observed to drop with rc=9).
- Probe 03 (`$SH -i -c exit`) is **retired**: GNU bash in WSL hangs on any
  interactive invocation without a tty (`bash --norc -i -c exit </dev/null`
  blocks forever) — environment-bound, no comparable measurement possible.

## Environment record (exact)

| Item | Value |
|---|---|
| rubash binary | `target/debug/rubash.exe`, 16067072 bytes, debug profile |
| rubash commit | `9bf2df9eed02ec77883ae2981e9cd769b6839b0c` (wt4/perfsuite base) |
| rubash banner | GNU bash, version 5.3.0(1)-release (x86_64-pc-msys) |
| GNU baseline | `/usr/local/bin/bash`, GNU bash 5.3.0(1)-release (x86_64-pc-linux-gnu), owner-compiled |
| WSL kernel | Linux 5.10.16.3-microsoft-standard-WSL2 x86_64 |
| Outer harness shell | Git Bash 5.2.37(1) (`D:/Git/bin/bash.exe`), MINGW64_NT-10.0-19044 |
| Env hygiene | `BASH_ENV`, `WINUXSH_ROOT` unset; `rm`/`rmdir`/`unlink` functions unset |
| GNU-side env | `PATH=/usr/local/bin:/usr/bin:/bin`, `HOME=/tmp/rubash-perf-home.$$` |
| rubash-side env | inherited Git Bash PATH (msys + Windows dirs), user HOME |
| Date | 2026-09-27 (UTC timestamps in the TSV headers) |

## Baseline table (median ms; ratio = rubash / GNU; bold = >= 10x)

| probe | shape | rubash ms | GNU ms | ratio | status |
|---|---|---:|---:|---:|---|
| 01-startup-empty | startup floor (`exit 0`) | 92 | 7 | **13.1x** | OK |
| 02-startup-fndef | startup + 1 fn def/call | 89 | 7 | **12.7x** | OK |
| 04-loop-true-builtin-x2000 | builtin `true` loop | 1048 | 20 | **52.4x** | OK |
| 05-arith-x5000 | `(( i += 1 ))` loop | 648 | 13 | **49.8x** | OK |
| 06-strconcat-x5000 | `s+=x` loop | 1915 | 25 | **76.6x** | OK |
| 07-fncall-noop-x5000 | no-op fn call loop | 3480 | 37 | **94.1x** | OK |
| 08-cmdsub-true-x1000 | `x=$(true)` loop | 477 | 382 | 1.2x | OK |
| 09-external-uname-x300 | external spawn loop | 316 | 215 | 1.5x | OK |
| 10-pathmiss-x100 | `command -v` miss loop | 261 | 24 | **10.9x** | OK |
| 11-pipeline-yes-head | `yes \| head -100000` | 221 | 8 | **27.6x** | OK |
| 12-pipe-echo-read-x2000 | `echo \| while read` x2000 | 5873 | 1030 | 5.7x | OK |
| 13-readloop-gen-x2000 | gen \| while read | 2549 | 24 | **106.2x** | OK |
| 14-glob-srcrels-x100 | `src/*/*.rs` glob loop | 3365 | 43665 | 0.1x | OK (env-bound, see notes) |
| 15-expansion-x5000 | `${a#..}`/`${a%%..}` mix | 4492 | 59 | **76.1x** | OK |
| 16-parse-flat8000 | exec 8000 assignments | 2219 | 18 | **123.3x** | OK |
| 17-parse-flat8000-n | `-n` parse only | 510 | 13 | **39.2x** | OK |
| 18-nested-brace-nst1-d200 | one-line nested braces D=200 | 579 | 6 | **96.5x** | OK |
| 19-nested-brace-nst2-d200 | two-line nested braces D=200 | 91 | 6 | 15.2x | RC2 (rubash parse FAIL) |
| 20-as-fn-mkdir-p-rep40 | autoconf unit x40 defs | 521 | 8 | **65.1x** | OK |
| 21-configure-head1374-n | `-n` bash configure head (1374 L) | 3211 | 9 | **356.8x** | OK |
| 22-configure-full-n | `-n` full bash configure (24753 L) | >120000 | 46 | >2600x | TIMEOUT (rubash) |
| 23-nvm-parse-n | `-n` nvm.sh v0.40.8 | 18748 | 22 | **852.2x** | OK |
| 24-nvm-load | source nvm.sh v0.40.8 | 14402 | 38 | **379.0x** | OK |
| 25-yes-head-read | `yes \| head \| while read` | - | 650 | - | TIMEOUT (rubash; #206 shape; **see #243 correction below — not a hang**) |

### Reading the numbers

- **Worst rows**: 23 nvm-parse 852x, 24 nvm-load 379x, 21 configure-head1374
  357x, 22 full configure (no finite number; >120 s vs 46 ms), 13 read-loop
  106x, 16 flat parse 123x, 18 nested-brace 96.5x, 07 fn-call 94.1x.
- **Near parity**: 08 `$(true)` (1.2x) and 09 external spawn (1.5x) — rubash's
  subshell/spawn machinery is competitive; the cost lives in per-command
  interpreter work, not process creation.
- **12 (echo|read) at 5.7x** vs **13 (read loop) at 106x** shows the builtin
  pipeline itself is only ~6x; the read builtin / simple-command dispatch is
  the amplifier.

### Measurement caveats (keep honest)

- Every timed run is wrapped in `timeout -k 5`. On Windows this adds a
  constant per-run overhead (msys `timeout.exe` spawn) to every rubash row —
  most visible in 01/02 (~10-20 ms of the 92 ms); GNU pays ~1 ms. Startup
  ratios are therefore slightly overstated; all ratios are conservative.
- **14-glob is environment-bound**: the GNU side globs through WSL drvfs
  (/mnt/d), which costs ~43 s/run vs rubash's native NTFS 3.4 s. The 0.1x
  ratio measures the filesystem, not the shells. Keep the row for
  rubash-side regression tracking only.
- 08/09 mix OS process models (Linux fork+exec vs Windows CreateProcess);
  both land near parity here, so no rubash action is implied.
- Probe 22: rubash cannot fully `-n` bash's own configure — on the CRLF
  working-tree copy it aborted at line 5345 after ~27 s (syntax error in a
  multi-line `as_fn_error` message string; GNU parses it), and on the LF copy
  it exceeds the 120 s budget. The GNU number (46 ms, rc=0) is a full parse.
- Probe 19: rubash (9bf2df9e) fails to parse ANY multi-line nested-brace
  layout (2-line form at any depth >= 2; one-`{`-per-line form at depth >= 4);
  GNU runs both in ~6 ms. The row is a canary: RC2 until that parse gap
  closes, then it becomes a timing row.

## Correction (2026-09-27, wt4/perffix lane): probe 25 is NOT a hang (rubash#243)

Reproduced from script files at BOTH 9bf2df9e (the commit the baseline was
recorded on) and 7dfdaeb0 (this lane's base), debug binaries, `timeout -k 5`:

| shape | 9bf2df9e debug | 7dfdaeb0 debug | 7dfdaeb0 release |
|---|---|---|---|
| `yes \| head -100000 \| while read -r l; do :; done` | rc=0, 40.5 s | rc=0, 40.6 s | rc=0, 6.0 s |
| `yes \| head -100000 \| while read; do :; done` | — | rc=0, 23.3 s | rc=0, 3.5 s |

RSS stayed flat at ~10 MB for the whole run (the pre-#206-fix blowup was GBs).
The #206 fix (62c7e5fd `execute_external_prefix_concurrently`) holds; the
baseline TIMEOUT row was the harness's `PERF: runs=3 timeout=20` truncating a
~23-41 s debug run (release ~3.5-6 s, matching the 62c7e5fd verification).
The read-tail loop cost — the probe-13 family, ~0.4 ms/line in debug — is
the amplifier. No bisect was needed (nothing regressed); the probe timeout
is now 120 s (benchmarks/25-yes-head-read.sh) and the class is pinned by
`perf_canary_yes_head_read_terminates` in tests/regression.rs (N=10000,
60 s bound). #243 is a measurement artifact, not a rubash bug.

## Corpus provenance (fetched / vendored, offline-capable)

| Corpus | Source | Version | License |
|---|---|---|---|
| `benchmarks/corpus/nvm.sh` | https://github.com/nvm-sh/nvm | v0.40.8 (tag), commit a885b885fef16fac4bc544188fb25e9e37ae83e8 (2026-09-21), 172906 bytes, vendored verbatim | MIT |
| `third_party/bash/configure` | vendored GNU bash 5.3 source tree (bash.git @b4608166, Bash-5.3 patch 15) — referenced in place; harness makes an LF copy under `target/perf-corpus/` (working-tree copy is CRLF per core.autocrlf) | 24753 lines, autoconf-generated | GPLv3+ (already vendored) |
| probes 18/19/20 shapes | issues #176 (nst1/nst2) and #178 (as_fn_mkdir_p rep-40) — regenerated from issue descriptions; unit extracted verbatim from configure lines 325-368 | — | — |
| 51-unit corpus | NOT PRESENT in this worktree at lane start (sibling corpus lane had not landed); not included | — | — |

## Coverage vs existing issues (all previously-filed [perf] issues are CLOSED)

| Family | Probes | Prior issues (closed) | New filing |
|---|---|---|---|
| Parse throughput (real scripts) | 16, 17, 18, 20, 21, 22, 23, 24 | #130, #155, #176, #178 | **#241** |
| Execution hot path | 04, 05, 06, 07, 10, 13, 15 | #156, #157, #186 | **#242** |
| Pipeline streaming | 11, 12 | #157, #206 | folded into **#242** |
| Startup | 01, 02 | #158 | folded into **#242** |
| `yes \| head \| while read` hang | 25 | #206 (closed) | **#243** |

## Side-findings (compat divergences observed while building probes — not fixed in this lane)

1. `{{ ... }}` without spaces: GNU lexes `{{` as a literal word (syntax
   error); rubash accepts it as nested reserved braces.
2. `{ :;;}`: GNU rejects `;;` inside a brace group (case token); rubash
   accepts it.
3. Multi-line nested braces do not parse in rubash at all (probe 19 note).
4. `rubash -i -c exit <file>` hangs (trailing file arg after `-c` is consumed
   as interactive input); `bash -c exit file` treats it as `$0`.
5. rubash `-n` on bash's configure aborts at line 5345 (multi-line
   `as_fn_error` message string) — rc=2 on CRLF copy, rc=1/hang on LF copy.

## Perffix lane results (2026-09-27, wt4/perffix on 7dfdaeb0)

### Side-findings 1+2 are FIXED in this lane (rubash#241 sub-items)

- `}}`/`}x` now lex as literal WORDS: GNU syntax.h:29-30 puts `}` in
  neither `shell_meta_chars` nor `shell_break_chars`, and
  CHECK_FOR_RESERVED_WORD (parse.y:3168, exact STREQ at parse.y:3174-3175)
  yields the reserved `}` only for the exact one-character word in an
  acceptable position outside a case pattern (parse.y:3177, 5899).
  `{{ echo hi; }}` now runs both words as commands (GNU parity: 127,
  two `command not found` lines); `echo } }x }}` prints `} }x }}`.
- `;;`/`;&`/`;;&` reaching the main list loop is now unconditionally a
  syntax error (GNU parse.y:3711-3717 SEMI_SEMI lexing; grammar admits
  them only in case_clause_sequence, parse.y:1237-1247). `{ :;;}` and
  `{ ;; :; }` reject with GNU's exact message.
- The whole empty-separator class is fixed per GNU's list grammar
  (parse.y:1264-1290): a `;`/`&` must follow a completed command
  immediately — leading `;`, `; ;`, `: <newline> ; :`, `: & & :`,
  `if :; then :; ; fi` all reject; `{ :; }`, `{ : ; }`, trailing `;`,
  `cat <<EOF ; :`, `case x in a) : ; esac`, `for ((i=0;;i++))` stay
  legal. Compound-body reparses now thread the enclosing source text so
  the error echoes the full physical line (GNU parse.y:6813-6824
  print_offending_line).
- Verification: 35-shape GNU-diff matrix (both sides from script files,
  rc + full stdout/stderr), 34 PASS; the 1 non-PASS is a pre-existing
  background-job stdout-ordering divergence (`echo bg &` output arrives
  after the next command's) that this lane did not touch. Pinned by
  `parse_acceptance_gnu_separator_and_brace_word_rules` in
  tests/regression.rs. Raw matrix: `target/issue-suites/results/perffix/`.

### Sub-item dispositions (not this lane's fixes — duplicate evidence)

- Side-finding 3 (multi-line nested braces) and 5 (configure `-n`): the
  failing constructs are brace-group scans spanning newlines — the same
  brace_scan_cache/skip_brace family the regfix lane was dispatched for;
  regfix has produced no commits yet (wt4/regfix == 1f30c090). The
  configure:5345 abort isolates to `{ { ...<newline>...;}\ncmd; } ;; #(`
  inside a case arm (GNU parses, rubash rc=2); depth-3+ two-line
  `{ { {\n:; } } }` fails while depth-2 passes on this base. Evidence:
  `target/issue-suites/results/perffix/{asfn5345,n2}.sh` runs.
- NEW evidence for regfix: `tests/regression.rs
  upstream_suite_slice_snapshots` (comsub2) is RED AT BIRTH — it fails
  identically at 1f30c090, the commit that created the snapshot (nested
  funsub `echo ${ echo X${ echo nested; }Y; }` → "syntax error near
  unexpected token `}'", GNU prints `XnestedY`). Reproduced in a scratch
  worktree with third_party populated. regfix's "snapshot 甄别" item.
- Side-finding (new, unfixed): a glued closer `}}` — `f() { :; }}` — GNU
  reports `unexpected end of file from `{` command` (the word `}}` never
  closes the group) while rubash treats the first `}` as the closer.
  skip_brace counts any `}` as a closer; GNU needs the exact `}` word.

### Hot-path profile (Task 3; RUBASH_EXEC_PROFILE + scratch counters, since removed)

- **Re-tokenization is ELIMINATED as a cost center for the interpreter
  probes**: probes 04/05/15 run 10-15 tokenize calls (~0 ms) and 3
  parses total per script — ASTs are built once and executed from AST.
  The queued "批式 tokenizer 每趟重词法化 → 增量读取器" deep-subsystem
  item does not cover these probes' cost.
- Top cost centers (debug, relative): (1) assignment execution path
  (`execute_empty_words_command`, 28% of probe 15 wall — mostly
  legitimate RHS parameter-expansion work); (2) `expand_command_words`
  (27% of probe 04); (3) the per-command preamble fixed costs —
  comsub/extglob diagnostic re-scans (6-16%), current-line env insert,
  alias/redirect/function-lookup checks (1-6% each). Arithmetic
  evaluation itself is 23% of probe 05 and already has the rubash#156
  fast admission.
- Landed win: admission whitelists on the two per-command diagnostic
  re-scans (`raw.contains('$')||raw.contains('`')` before the comsub
  DFA; `raw.contains('(')` before the extglob DFA) — provably
  equivalent (comsub_residuals only reports residuals introduced by
  those characters), ~5% off the scans phase, ≤1% wall on these probes.
  Everything else measured (validate/alias/stdin-preexpand/strip/
  function-lookup/assignment-classify segments) is ≤14 ms per probe —
  the per-command overhead is distributed, not concentrated; no single
  safe structural win of >5% exists in this layer without semantic
  rework of expansion/assignment internals.

## perf2 round (2026-09-27, wt5/perf2 on 86818357)

Second attack on #241/#242, building on perffix's profile (re-tokenization
eliminated for hot probes; costs = per-command expansion work + per-pass
O(buffer) re-scans). All numbers: debug build, median of 5 timed runs,
`scripts/run-perf-suite.sh --probe N --runs 5`, same harness on both sides;
the GNU column was re-measured this round (WSL load differs from baseline
day by a few ms), so compare ratio-to-ratio and rubash-ms-to-rubash-ms.

### Landed changes (4 files, provably-equivalent admissions; see commit)

1. **`src/builtins/set/options.rs` — allocation-free `shell_option_enabled`.**
   The per-command/per-word consults (`is_brace_expand_enabled` during word
   expansion; errexit/xtrace/noexec preambles) built a `format!` key per
   call. Stack-buffer key, same key bytes, same lookups (long names fall
   back to the old path). Measured: probe 07 94.1x -> 52.3x, probe 13
   106.2x -> 37.8x (each command consults the option table several times).
2. **`src/executor/command_prepare.rs` — provably-trivial literal-word fast
   path in `expand_command_word`** (rubash#117 whitelist rule): when
   `raw == word` and every byte of both is in a set that excludes every
   syntax the expansion pipeline reacts to (`$ \` ' " { } ~ * ? [ ( ) < > =`
   whitespace, and all non-printable/carrier bytes), the word expands to
   itself (GNU expand_word_internal produces it byte-identically; no brace,
   no glob metachar, no expansion fragment exists). Everything else falls
   through to the full pipeline unchanged.
3. **`src/executor/command_prepare.rs` — brace gate**: `expand_braces(r)`
   (Vec+String) now runs only when `r.contains('{')` (braces.c
   find_first_valid_brace requires one).
4. **`src/lexer/skip.rs` — O(span^2) -> O(span) in
   `skip_parenthesized_unit_corrected`**: the per-character
   `chars[index..].iter().collect::<String>()` suffix existed only to feed
   `case_pattern_starts_with_esac_rest`, which reads `rest` only on the
   `word == "esac"` + `)`/`|` arm; it is now materialized only for that
   shape (byte-identical value when materialized). nvm.sh -n: 2.36s -> 6ms
   in `unclosed_array_subscript_line` (44 comsub spans).
5. **`src/lexer/mod.rs` — checkpoint for the per-line `has_unclosed_quotes`
   rescan** (brace_scan_cache discipline): the tokenizer re-runs the
   captain's scanner over the whole accumulated logical line per physical
   line. The scanner's answer (`single || double || ansi_single`,
   continuation.rs:856) can only change when the appended line contains
   `\ " ' $ \`` (all other bytes fall to the no-op arm;
   `brace_join_fast_path_line` inert lines are a superset). The cached
   answer is reused for inert appends and invalidated at every non-append
   mutation (IFS_GLUE insert, backslash pop, comsub-heredoc rotation,
   flush). Tokenize loop on nvm.sh: 6.71s -> 5.31s in that scan.

### Ratios (perf2 round)

| probe | baseline rub/GNU | perf2 rub/GNU | rubash ms |
|---|---|---|---:|
| 01-startup-empty | **13.1x** | **18.0x** | 92 -> 90 (flat; GNU 5ms this round) |
| 02-startup-fndef | **12.7x** | **12.2x** | 89 -> 73 |
| 08-cmdsub-true-x1000 | 1.2x | 1.0x | 477 -> 327 (parity) |
| 09-external-uname-x300 | 1.5x | 1.0x | 316 -> 194 (parity) |
| 04-loop-true-builtin-x2000 | **52.4x** | **44.2x** | 1048 -> 575 |
| 05-arith-x5000 | **49.8x** | **37.6x** | 648 -> 451 |
| 06-strconcat-x5000 | **76.6x** | **57.8x** | 1915 -> 1330 |
| 07-fncall-noop-x5000 | **94.1x** | **52.3x** | 3480 -> 1673 |
| 10-pathmiss-x100 | **10.9x** | **10.4x** | 261 -> 218 |
| 11-pipeline-yes-head | **27.6x** | **29.4x** | 221 -> 235 (noise) |
| 12-pipe-echo-read-x2000 | 5.7x | 3.6x | 5873 -> 3458 |
| 13-readloop-gen-x2000 | **106.2x** | **37.8x** | 2549 -> 908 |
| 15-expansion-x5000 | **76.1x** | **63.9x** | 4492 -> 3643 |
| 16-parse-flat8000 | **123.3x** | **123.6x** | 2219 -> 2102 |
| 17-parse-flat8000-n | **39.2x** | **32.8x** | 510 -> 394 |
| 18-nested-brace-nst1-d200 | **96.5x** | **112.0x** | 579 -> 560 (GNU 5ms this round) |
| 20-as-fn-mkdir-p-rep40 | **65.1x** | **59.8x** | 521 -> 478 |
| 21-configure-head1374-n | **356.8x** | **358.2x** | 3211 -> 3224 (flat) |
| 23-nvm-parse-n | **852.2x** | **719.8x** | 18748 -> 12237 |
| 24-nvm-load | **379.0x** | **228.6x** | 14402 -> 8002 |

No probe ended the round >2x better; #241 and #242 stay open (re-scope
below). Allocation profile that motivated the hot-path work (scratch
counting allocator, removed): probe 15 ran ~330 allocations per command —
95 per assignment RHS (`${a#..}` walker re-entered the whole expansion
pipeline via `expand_word_mut_with_context` reconstruction: 2.50M allocs /
1.29s of 4.5s wall), 33-65 per word. The landed word fast path + shopt +
brace admissions remove the churn for literal words; the walker-recursion
restructure remains the next hot-path target but touches SubXpassFrame
memoization semantics and was not attempted under the zero-behavior-change
gate.

### What remains (for the captain — continuation.rs is exclusive)

nvm.sh -n now spends ~4.5s in ONE `has_unclosed_quotes` call plus ~5.3s in
the per-line calls (both continuation.rs):

- **continuation.rs:762** (`has_unclosed_quotes`, `${` arm):
  `let body: String = chars[index + 2..].iter().collect();` copies the
  ENTIRE remaining input per `${` (nvm.sh: 1644 occurrences -> O(n^2);
  measured 20K->40K->80K->173K prefixes: 40/185/878/4437ms). Same-shape
  copies at ~783-791. Fix shape: pass `&chars[index+2..]` slices (the
  dolbrace scanner takes &str; a slice view over the already-collected
  `chars` Vec avoids the per-`${` copy).
- Same function powers the per-line tokenize-loop scan (mod.rs), so one fix
  improves both the whole-input prescan and the loop; my
  `unclosed_quotes_cache` (mod.rs) already skips inert-line re-calls.
- 21-configure-head1374-n is flat because m4sh lines are dense (few inert
  lines, few long comsub spans) — its cost is entirely the two
  continuation.rs scans above.
- The `-n` reader itself (parse phase) is 0.9s of 12.2s; tokenize_plain
  re-tokenization 0.8s; ast-walk under `-n` is negligible (0.4ms).

### perf2 side-finding (pre-existing, reproduced at 86818357)

`v=$(case z in z) printf 'z\n' ;; esac)` stores `zn` (the `\` is dropped
somewhere in the case-in-comsub body path); GNU stores `z`. Repro:
`/tmp/pr6.sh` shape above; base build reproduces byte-identically, so it is
NOT from this round. Owner: comsub body extraction family.

## perf3 round (2026-09-28, wt7/perf3 on 92de75bd)

Third attack at #241/#242/#281 (the #281 umbrella: walker `${`-arm recursion,
batch-tokenizer complete-command-boundary checkpoint, and the
captain-exclusive continuation.rs cost centers). All numbers: debug build,
median of 3 timed runs via `scripts/run-perf-suite.sh` (same harness both
sides); the GNU column re-measured this round, so compare
ratio-to-ratio and rubash-ms-to-rubash-ms. Environment: same host, CPU load
elevated by unrelated processes during parts of the round — A/B medians
below were verified with back-to-back alternating runs of two binaries
where the delta was small.

### Landed changes (8 files; scratch instrumentation removed before commit)

1. **Walker `${`-arm fragment resolution — no pipeline re-entry**
   (`parameter_words.rs expand_braced_parameter_fragment_in_word`,
   `embedded_mutations.rs` arm). GNU anchor: `subst.c:11229
   expand_word_internal()` is a single pass whose `${` arm calls
   `parameter_brace_expand` (subst.c:9777) inline; the old reconstruction
   `expand_word_mut_with_context(&format!("${{{name}}}"))` re-ran the whole
   pipeline preamble per fragment (fresh SubXpassFrame scope, the `:=`/`=`
   assignment pre-scan, funsub/whole-word routing scans — ~58 allocations
   per `${}` fragment, 1.15M allocs / 769ms of probe 15's 3.4s wall).
   Admission whitelist (rubash#117): the fragment name contains no `=`
   (both assignment pre-scan splits fail), does not start with a funsub
   introducer (whitespace/`|`), and `braced_parameter_spans_whole_word`
   holds for the synthetic word — under those conditions the re-entry's
   routing target was provably `expand_quoted_parameter_word_mut(synthetic,
   context)`, which the fast path now calls directly. SubXpassFrame memo
   semantics: the nested fragment's cross-pass entries now land in the
   ENCLOSING word's frame instead of a private frame dropped on return —
   keys are (word-context id, fragment path, text) and the walker's site
   guard has already extended the path with the fragment's ordinal, so
   sibling fragments cannot false-hit while re-probes of the same fragment
   now dedup (the memo's documented purpose).
2. **`case_pattern_matches` literal-equality fast path**
   (`conditional/pattern.rs`): a pattern whose every byte is printable
   ASCII without any glob/extglob syntax byte matches by plain equality
   (GNU strmatch walks chars; the two `Vec<char>` stagings per call were
   this port's cost, not GNU's — the prefix/suffix removal loop paid them
   once per candidate boundary).
3. **`remove_matching_prefix/suffix` allocation-free boundary walk**
   (`parameter_decode.rs`): the `Vec<usize>` + `Box<dyn Iterator>` staging
   became a `Chain<Map<CharIndices>, Once>` used forward/reversed
   (DoubleEndedIterator) — byte-identical order, two allocations fewer per
   removal call.
4. **`expand_parameter_pattern_word` literal fast path**
   (`parameter_patterns.rs`): a pattern of printable ASCII bytes outside
   the quote/expansion/escape/sentinel set (`' " $ { } \` and backslash)
   carries no syntax any downstream step reacts to, so every one of them
   (masking, `${}` slotting, anchor marking, quote decoding, the embedded
   walk, the marker replaces) is the identity — one clone replaces ~8
   String builds plus a full walker pass per pattern.
5. **`has_unclosed_parameter_expansion` + `skip_braced_parameter_in_chars`
   zero-copy `${` scans** (`brace_scan.rs`): both carried the same
   per-`${` `chars[index + 2..].iter().collect::<String>()` copy of the
   ENTIRE remaining buffer as continuation.rs (nvm.sh: 1644 `${`
   occurrences → O(buffer²) per call). Both now slice `&chars[index..]`
   into the existing zero-copy `scan_braced_parameter_body_chars` API
   (rubash#185). NOTE: that API requires the `${` at slice[0]
   (`scan_braced_chars_from` verifies it) — slicing from index+2 makes
   every span look unclosed. Un-covering this also un-covered that the
   old join short-circuit (`brace_group_open || scan()`) was what kept the
   quadratic off nvm's hot path; the scan is now linear so the join can
   consult it unconditionally (the checkpoint's safety gate needs the
   true param state).
6. **Batch tokenizer complete-command-boundary checkpoint**
   (`scanner.rs LexerBoundaryState`, `mod.rs`): GNU reads its input once,
   token by token (parse.y:3557 read_token); the batch tokenizer re-lexed
   the WHOLE accumulated logical line after every appended physical line
   (nvm.sh `-n`: 5732 passes re-lexing 4.19MB). When a pass ends BETWEEN
   tokens — quotes, command substitutions, compound assignments and
   parameter expansions all closed (the huq/comsub/compound gates prove
   it) and no open `(`/`((` group or pending extglob split
   (`LexerBoundaryState::boundary_state` refuses those) — its token list
   and lexer state are exactly the full pass's prefix results, so the
   next pass resumes at the boundary and lexes only the appended tail.
   Safety gate: the appended line must contain no `}` byte — a `{` group
   still open in the prefix is emitted as a bare `{` Keyword plus
   individually-lexed body tokens and FOLDS into one token in exactly the
   pass where its `}` arrives (scanner.rs scan_token `{` arm), a fold a
   resumed lexer cannot perform for an opener before the checkpoint
   offset; every fold completion needs a `}` byte, so refusing the resume
   restores full re-lexing exactly on fold passes. Invalidation
   discipline = brace_scan_cache: every non-append mutation (IFS_GLUE
   insert, backslash-join pop, comsub-heredoc rotation, flush) plus
   `set -o posix` / `shopt extglob` value flips (the full pass would
   re-lex the prefix under the new mode).

### Ratios (perf3 round; baseline = this lane's own 92de75bd run)

| probe | base rub/GNU | perf3 rub/GNU | rubash ms |
|---|---:|---:|---:|
| 01-startup-empty | **13.1x** | **14.5x** | 92 -> 87 (noise band) |
| 02-startup-fndef | **12.7x** | **15.2x** | 89 -> 91 (GNU 6ms this round) |
| 04-loop-true-builtin-x2000 | **52.4x** | **41.7x** | 1048 -> 626 |
| 05-arith-x5000 | **49.8x** | **38.0x** | 648 -> 456 |
| 08-cmdsub-true-x1000 | 1.2x | 1.0x | 477 -> 358 (spawn parity retained) |
| 09-external-uname-x300 | 1.5x | 0.8x | 316 -> 188 (parity retained) |
| 15-expansion-x5000 | **56.8x** | **54.9x** | 3407 -> 3293 (harness; back-to-back A/B medians 3545 -> 3418 under load; walker-guard cost 769 -> 508ms, allocs 3.58M -> 3.04M) |
| 16-parse-flat8000 | **123.3x** | **125.2x** | 2219 -> 2253 (flat) |
| 17-parse-flat8000-n | **39.2x** | **27.9x** | 510 -> 390 |
| 18-nested-brace-nst1-d200 | **96.5x** | **99.3x** | 579 -> 596 (flat; GNU 6ms) |
| 20-as-fn-mkdir-p-rep40 | **65.1x** | **54.0x** | 521 -> 486 |
| 21-configure-head1374-n | **395.3x** | **383.1x** | 3558 -> 3448 (flat — dominated by continuation.rs prescan) |
| 23-nvm-parse-n | **623.6x** | **644.9x** | 12472 -> 12254 (flat — dominated by continuation.rs) |
| 24-nvm-load | **379.0x** | **208.1x** | 14402 -> 8116 |

Side win: the `bash_completion -n` canary (regression
`canary_bash_completion_bash_n_when_corpus_present`) runs 1443ms -> ~700-800ms
with the exact GNU error retained (line 1820 extglob `(`).

### nvm -n / configure -n decomposition (scratch phase timers, removed)

On 92de75bd the 12.4s nvm -n spend was: 4.54s ONE whole-input
`has_unclosed_quotes` prescan + 5.33s per-line `has_unclosed_quotes` calls
(both continuation.rs, captain-exclusive) + 0.98s tokenize_plain
re-tokenization + ~1.6s parse/reader. After perf3's committed changes
(without touching continuation.rs): 12.25s (645x). With the captain's
PROPOSED continuation.rs slice fix (below) temporarily applied on top:
**2607ms = 137.2x** (probe 23) and **2606ms = 289.6x** (probe 21); probe 22
(configure full -n) still exceeds its 120s budget (no output, rc=124 at
100s — a parse-phase super-linear cost distinct from the `${` scans).

### Acceptance re-scope (honest numbers)

The <60x targets for nvm -n (from 720x) and configure-head (from 357x) are
NOT reached this round and cannot be from this lane alone: 9.9s of nvm -n's
12.4s lives in `continuation.rs` (captain-exclusive), and even with the
proposed continuation.rs diff applied nvm -n is 137x — the remaining ~2.6s
splits ~1.0s lexer (now: comsub scan ~0.2s, huq ~0.12s, tokenize ~0.36s)
and ~1.6s parse/reader phase, which is the next deep system (GNU does the
whole parse in 19ms). Re-scope: #241/#242/#281 stay open with these
numbers; #281's umbrella items (walker memo + boundary checkpoint) are
landed, its continuation.rs cost centers remain captain-owned.

### Verification (this round)

- `cargo build` 0 warnings; `cargo check --tests` clean; `cargo test --lib`
  482/482; regression 24/24 (54-92s, was 91s at base).
- 120-case `${...}` parameter-expansion GNU-diff matrix (probe files under
  `target/issue-suites/results/perf3/param-matrix*.sh`): byte-identical
  stdout/stderr/rc vs WSL GNU 5.3.0 (only the $0-path prefix in two stderr
  lines differs — invocation artifact, both shells print their own $0).
  Covers plain/pattern/substring/transform/case-mod/operator forms, the
  `=`-bearing fragments excluded from the fast path, nocasematch, extglob,
  nameref/indirect, nested patterns, arithmetic offsets.
- Two multi-line logical-line matrices (`lex-matrix*.sh`, same dir):
  byte-identical incl. rc — function bodies, nested groups, inline and lone
  `}` closes, `${` joins, backslash continuations, multi-line strings,
  compound arrays, subshells, `((`/if/elif/for/until/case, heredocs inside
  groups, comments at group top, `set -o posix` flips.

### Captain's proposed continuation.rs diff (rubash#281, NOT applied —
captain-exclusive; measured by temp-apply + revert)

The `${` arm of `has_unclosed_quotes` (continuation.rs:762) and the `${`
skip of `comsub_residuals` (continuation.rs:1009) copy the ENTIRE
remaining input per `${` (`let body: String = chars[index + 2..]
.iter().collect();`) — O(n²) with nvm.sh's 1644 occurrences. The
zero-copy `scan_braced_parameter_body_chars` API already exists
(dolbrace.rs, rubash#185). Verbatim diff:

```diff
--- a/src/lexer/continuation.rs
+++ b/src/lexer/continuation.rs
@@ has_unclosed_quotes, `${` arm (~line 762)
         if ch == '$' && !single && chars.get(index + 1) == Some(&'{') {
-            let body: String = chars[index + 2..].iter().collect();
+            // rubash#281: zero-copy body view (rubash#185 API); the String
+            // copy re-collected the ENTIRE remaining input per `${` (nvm.sh:
+            // 1644 occurrences -> O(n^2)). Slice INCLUDES the `${` opener:
+            // scan_braced_parameter_body_chars requires it at position 0.
+            let body = &chars[index..];
             if !double {
                 let context = crate::lexer::dolbrace::BraceContext {
                     outer_double_quote: false,
                     posix: false,
                     replacement_context: false,
                     initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                 };
                 if let Some(scan) =
-                    crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
+                    crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
                 {
-                    index += 2 + body[..scan.end].chars().count();
+                    index += 2 + scan.end;
                     comment_start = false;
                     continue;
                 }
@@ same function, double-quoted posix loop (~line 785)
                     if let Some(scan) =
-                        crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
+                        crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
                     {
-                        index += 2 + body[..scan.end].chars().count();
+                        index += 2 + scan.end;
                         comment_start = false;
                         closed = true;
                         break;
                     }
@@ comsub_residuals, `${` skip (~line 1009)
         if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'{') {
-            let body: String = chars[index + 2..].iter().collect();
+            let body = &chars[index..];
             let context = crate::lexer::dolbrace::BraceContext {
                 outer_double_quote: false,
                 posix: false,
                 replacement_context: false,
                 initial_state: crate::lexer::dolbrace::DolbraceState::Param,
             };
-            if let Some(scan) = crate::lexer::dolbrace::scan_braced_parameter_body(&body, context) {
-                index += 2 + body[..scan.end].chars().count();
+            if let Some(scan) =
+                crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
+            {
+                index += 2 + scan.end;
             } else {
                 index += 2;
             }
```

Measured effect (temp-applied on the perf3 state, then reverted):
prescan 4540ms -> 27ms, per-line huq 5325ms -> 122ms, nvm -n
12254ms -> 2607ms (645x -> 137x), configure-head 3448ms -> 2606ms
(383x -> 290x). The `&chars[index..]` slice (WITH the `${`) is load-bearing:
slicing from index+2 makes `scan_braced_chars_from` reject every span
(dolbrace.rs requires `chars[0] == '$'`), which reports every line with a
`${` as unclosed — that mistake was caught by the regression canaries this
round, not by timing.

## Attribution profiling (2026-09-28, owner-mandated measurement infrastructure)

Wall-clock tables above answer *which scenario* is slow; they cannot answer
*where inside the binary* the time goes. The attribution layer:

- **Tool**: [`samply`](https://github.com/mstange/samply) — sampling
  profiler, Firefox Profiler JSON output, MSVC symbol support.
  Install: `cargo install samply --locked`.
- **Driver**: `scripts/profile-startup.sh rubash|niu <args…>` (or
  `--script file.sh`) — records one run, saves
  `target/perf-profiles/<kind>-<name>-<ts>.json`. Open the JSON at
  https://profiler.firefox.com (Import) for the interactive flame graph,
  or `samply load <file>` for the local viewer.
- **Reading it**: the flame graph's x-axis is wall time; the tallest
  self-time columns under `main` are the fix targets. For
  `niu -C 'echo ok'` the interesting band is everything between process
  start and the rc-load completion (engine init, rc sourcing, OMB load,
  first PROMPT_COMMAND) — compare against a WSL GNU bash recording of the
  same OMB load to see the GNU-side profile shape.
- **Rules**: never measure a stale binary (build first); close other
  build jobs (parallel cargo builds skew samples); keep the produced JSON
  under `target/perf-profiles/` (gitignored) and record the numbers that
  matter in this file, not the raw profile.
## ombperf round (2026-09-28, wt9/ombperf on 847ab883): oh-my-bash load

Goal: OMB (agnoster) load in the user's real HOME is engine-bound, not
component-count-bound (three OMB-trim variants all 1.43-1.46s). Fix the
engine's per-op fixed costs. All numbers release build, median of 5,
Windows subprocess wall (python time.perf_counter) unless noted.

### GNU anchors (WSL 5.3.0, /usr/local/bin/bash, script-file probes)

- floor `bash --norc -i -c exit` (pty `script -qec`): 22 ms
- OMB load (LF copy, ext4): inner 644-695 ms
- OMB load (LF copy, /mnt/d NTFS via drvfs): inner 812-905 ms
- GNU cannot parse the CRLF originals; LF-converted copies used for
  content parity (conversion verified with od).

### Engine-side before/after (rubash.exe --rcfile <rc> -i -c "exit 0")

| metric | baseline 847ab883 | after 3 commits |
|---|---:|---:|
| floor `-c exit 0` | 21 ms | 21 ms |
| full OMB load, LF copy on D: | 1436 ms | ~828 ms |
| full OMB load, real CRLF on C: | 1424 ms | ~820 ms |
| inner `source oh-my-bash.sh` (LF / CRLF) | 1093 / 1090 ms | 787 / 780 ms |
| ConPTY (winpty backend) engine load | 5490 ms | 5161 ms |

### niu side (scratch niubash release, [patch] -> this worktree)

- `niu -c exit 0` floor: 160 -> 147 ms
- `niu -C exit 0` (real HOME, rc + OMB + hooks): 1919 -> 1484 ms
- `niu -C 'source OMB' --norc` inner: ~1090 -> 795 ms (median 790-803)
- ConPTY `niu -C exit 0`: 6162 ms median (pty substrate adds ~4.5 s over
  subprocess for BOTH baseline and current — winpty backend cost, not
  engine; real-terminal delta baseline->current ~-330 ms by engine ratio)

### Commits (each with GNU citation + numbers)

1. `f72558a1` perf(assoc): content-addressed parse memo. GNU assoc.c:68
   assoc_insert / hashlib.c:318 hash_insert are O(1); the rendered-string
   storage made every element op a full re-parse (~244 us/call, O(n^2)
   fills). OMB assoc fill 126.9->39.9 ms/call, key-copy 335.4->73.7 ms;
   load 1436->926 ms; profiled assoc parse total 4999->12 ms.
2. `25da547d` perf(lexer): skip duplicate comsub scan at the join gate.
   GNU parse.y:3557 read_token is a streaming reader (never re-scans the
   accumulated buffer); the join loop scanned it twice per physical line.
   omb-prompt-base.sh tokenize 93.8->57.9 ms; load 926->828 ms.
3. `3e1ec3b1` perf(arrays): same memo class for indexed storage
   (array.c:516 array_insert O(1) append). Element-write parse sub-step
   5.2->0.1 us; a[1]=$i shape 155.1->145.1 us (rest is expansion
   machinery, below).

### Semantics gate

PS1 md5, `declare -p` md5s of _omb_spectrum_fg / FX / FG, function count
(281), alias count (33): byte-identical baseline vs current (binaries
built from git-archive 847ab883 and wt9/ombperf HEAD). cargo test --lib
490/490; cargo fmt --check clean.

### Remaining engine hotspots (measured, not fixed this round)

- comsub_residuals / has_unclosed_quotes (continuation.rs, captain-
  exclusive): `input.chars().collect::<Vec<_>>()` per call over the whole
  accumulated logical line; 34 ms of omb-prompt-base's remaining 58 ms
  tokenize (n=984 calls, 2.5 MB re-scanned). Diff + data to captain.
- Pre-expansion validation walk: validate_command_parameter_expansions
  re-walks every word's `${` spans before the real expansion (GNU
  subst.c expands once, erroring inline); 264 ms inclusive in the load
  profile. Needs careful per-arm GNU alignment before removing.
- Per-command machinery floor: `:` costs 22 us vs GNU 1.8 us (expansion
  CommandNode clone, alias checks, dispatch chain); local3 161 us vs
  GNU 5.3 us and a[1]=$i 145 us vs 2.2 us live in the same expansion/
  declare-family machinery (expand_command_words is 110 us of the arridx
  op). Root-cause fix = borrow-based expansion refactor (deep subsystem).
- niu host layer: ~560 ms of `niu -C` beyond engine+floor (precmd hooks,
  gitstatus, completion refresh) — niubash repo, separate lane.

## perf4 round (2026-09-28, wt8/perf4 on 9a13e8f3)

Fourth attack at #241/#242, building on perf3 (walker memo, boundary
checkpoint, zero-copy scans) and the captain's landed continuation.rs
slice fix (15f94b9d — that landed AFTER perf3's re-scope, so this lane's
base already measures nvm -n at 2640ms/132x, not the 12254ms perf3 saw).
All numbers: debug build, same harness (`scripts/run-perf-suite.sh`,
median of 3-5 runs); the GNU column re-measured per round, and host load
varied during the day, so the table ALSO carries back-to-back A/B medians
against a pristine-base binary built in this round (same session, same
load) — that column is the honest apples-to-apples.

### Landed changes (13 files; scratch instrumentation removed before commit)

1. **Parse-phase owner: compound-body re-parse source cloning — 272MB on
   nvm -n.** `parse_body_with_diagnostics` (parse_loop.rs) built
   `ParseLoopOptions { diagnostic_text: source.map(str::to_string) }` for
   EVERY nested body parse (if/while/for/case/brace/function/subshell):
   nvm -n re-parses ~1500 nested bodies, each cloning the WHOLE 173KB
   script text (measured 272,154,044 bytes of clones). GNU anchor: GNU
   keeps one input string for the whole parse (parse.y shell_input_line
   family); the clone was this port's artifact. Fix:
   `ParseLoopOptions.diagnostic_text`/`source_text` and
   `ParseState.diagnostic_text` became `Rc<str>` threaded through every
   compound parser (if/loop/for/case/brace/subshell/function take
   `Option<&Rc<str>>`; readers keep `as_deref()`; the Rc clone is a
   refcount bump).
2. **Duplicate comsub scan in the tokenizer join loop eliminated.** The
   second `has_unclosed_command_substitution(&logical_line)` consult
   re-scanned the accumulated text the first consult (same iteration) had
   already scanned; between them the only mutation is the IFS_GLUE
   (\x1c) insert, a data byte to `comsub_residuals` (not `$` `` ` `` `(`
   `)` quote or `\`), so the answers are provably identical (GNU anchor:
   parse.y:3557 read_token computes its comsub state once per read).
3. **Admission gates + false-answer caches on the join-loop text scans**
   (mod.rs, same discipline as `unclosed_quotes_cache`):
   - comsub scan gated on `$`/backtick presence; a cached FALSE plus an
     appended line with neither opener byte stays false (every
     comsub_residuals opener starts with `$` or is a backtick).
   - compound-assignment scan gated on a literal `=(` byte pair (the
     opener is a word ending `=` whose next char is `(` — the bytes are
     adjacent whenever the scan can return true).
   - `${`-expansion scan gated on `$` (its only true exit is a `${` whose
     body scan failed); cached FALSE + `$`-free appended line stays false.
   Same invalidation set as `unclosed_quotes_cache` (IFS_GLUE insert,
   backslash pop, comsub-heredoc rotation, flush).
4. **Parse-loop per-token comsub-depth scan gated on `$(`**: the stray-`)`
   guard's `unclosed_command_substitution_depth(&tokens[i].raw)` ran
   `comsub_residuals` (with its Vec<char> collect) on EVERY word token;
   depth can only be non-zero when a literal `$(` byte pair exists.
5. **Fold-pass fast exits** (parse_loop.rs): each of the six fold passes
   (pipeline/time-pipeline/time-simple/inverted/and-or/background) is a
   whole-list identity map when no command carries its trigger field —
   gated with a list-level `any()` so trigger-free lists skip the
   rebuild (GNU anchor: the folds model grammar productions the token
   stream already proved absent, parse.y:1337-1352 etc.).
6. **Grouped-driver per-line completeness scan made incremental**
   (script_driver.rs `ConstructScanCheckpoint`): configure contains
   `set -o posix`, so it runs the GROUPED driver, whose gather loop
   re-ran `stdin_source_needs_more_posix` over the WHOLE accumulated
   group per physical line — O(group^2): the 3000-line prefix spent
   ~8.5s/11s right there. The checkpoint marks a byte offset at which
   every construct scanner (quotes/comsub/array/close-char) provably
   sits in its INITIAL state (all four probes false; a trailing `\`
   blocks the advance — an escaped boundary consumes the join separator
   differently), then scans only the appended tail per line. `expand_group_aliases`
   returns `Cow` (the per-line whole-pending String copy was itself
   O(group^2)); the alias expansion that ran twice per line is computed
   once; `stdin_line_ends_with_continuation` admits on the trailing byte
   (`ends_with_unquoted_backslash` re-collects the whole text per call —
   ~12s of the 6000-line prefix); grouping is byte-identical (verified:
   same 106 group boundaries on the 3000-line prefix).
7. **skip.rs `${`-arm zero-copy** (rubash#281 shape, my file):
   `command_substitutions_balanced` still carried the per-`${`
   `chars[index+2..].iter().collect::<String>()` copy of the ENTIRE
   remaining input — O(tail^2) per call (~13s of the 6000-line prefix).
   Fixed with the `scan_braced_parameter_body_chars` API exactly like
   the captain's landed continuation.rs pattern (slice INCLUDES `${`;
   `index += 2 + scan.end`).
8. **Per-command preamble env stamps made allocation-free when
   unchanged** (public_accessors.rs): `__RUBASH_CURRENT_LINE` /
   `__RUBASH_CMD_START_LINE` paid key+value String allocations per
   command (30k commands on probe 15); now rendered into a stack buffer
   and assigned only when the line moved.

### Ratios (perf4 round)

| probe | this-lane base (morning, quiet) | perf4 harness | back-to-back A/B vs pristine base (same load) |
|---|---:|---:|---:|
| 23-nvm-parse-n | 2640ms / 132.0x | **1643ms / 65.7x** | 3029 -> 1737ms (**-43%**) |
| 24-nvm-load | 3611ms / 95.0x | 2625ms / 51.5x | 3180 -> 2227ms (**-30%**) |
| 21-configure-head1374-n | 2554ms / 283.8x | 670ms / 67.0x | 2742 -> 636ms (**-77%**) |
| 22-configure-full-n | TIMEOUT (>120s) | **TIMEOUT (still >120s; >330s CPU, killed)** | prefix curve 3000: 11.0s->1.4s; 6000: 118s->5.5s; 9000: 107s->35.8s |
| 17-parse-flat8000-n | 390ms / 39.2x (perf3 day) | 304ms / 23.4x | 390 -> 268ms (**-31%**) |
| 16-parse-flat8000 | 2219ms (baseline day) | 2213ms / 130.2x | flat (execution dominates) |
| 04-loop-true-builtin-x2000 | 591ms / 45.5x | 748ms* / 53.4x | 612 -> 584ms (-4.6%) |
| 05-arith-x5000 | 450ms / 37.5x | — | 472 -> 475ms (flat) |
| 15-expansion-x5000 | 3191ms / 54.1x | — | 3299 -> 3237ms (-1.9%) |
| 08-cmdsub-true-x1000 | 1.2x | 1.4x | parity retained |
| 09-external-uname-x300 | 1.5x | 0.9x | parity retained |

*04's harness run landed during a host-load spike; the A/B column is the
reliable comparison.

### Acceptance scorecard (honest)

- **nvm -n < 80x: MET** — 132x -> 65.7x (1643ms vs GNU 25ms).
- **configure-full under 120s: NOT MET** — still TIMEOUT. The per-line
  completeness scans are now incremental, but configure's m4sh quoting
  makes the construct scanners report FALSE-POSITIVE open constructs,
  producing multi-thousand-line groups (one 8676-line group on the
  9000-line prefix) whose stuck tail still re-scans O(open stretch) per
  line. ROOT CAUSES (captain-family, with reproducers):
  1. `has_unclosed_quotes` false positive on the as_fn_mkdir_p quote
     nesting (`*\'*) as_qdir=\`printf "%s\n" "$as_dir" | sed
     "s/'/'\\\\\\\\''/g"\`;; #'(` — configure line 337): reproducer
     `target/perf4/q5.sh` (GNU rc=0 parses; the grouped driver's scans
     report open through the whole function; batch `-n` also accepts).
     continuation.rs — captain-exclusive.
  2. The tokenize-based keyword-stack stage (`stdin_source_needs_more`
     tokenize arm) keeps groups open on `if`/`case` false positives in
     the same stretch.
  With those fixed, groups stay small and the incremental machinery is
  already linear. Prefix-curve evidence recorded above.
- **04/05/15 each -25%: NOT MET** — landed only -2..-5% (04 -4.6%, 05
  flat, 15 -1.9% back-to-back). The per-command preamble env-stamp
  allocations were removed, but the remaining cost is the distributed
  expansion/assignment/loop machinery perffix and perf3 already profiled
  (no single >5% structural win without semantic rework). The perf3
  conclusion stands.
- **Spawn parity retained** — 08 at 1.4x, 09 at 0.9x.

### Verification (this round)

- `cargo build` 0 warnings; `cargo fmt`; `cargo test --lib` 486/486;
  `cargo test --test regression` 24/24; `cargo check --tests` and
  `cargo check --release --tests` clean.
- GNU-diff matrices (both shells from script files, rc + stdout +
  stderr): `target/issue-suites/results/perf4/matrix{1..4}.sh` — m1
  (multi-line joins: quotes/comsubs/backticks/arrays/heredocs/braces/
  backslash continuations/comments/posix) and m2 (grouped driver:
  `set -o posix` + functions/case/if/for/while/heredoc/brace) byte-
  identical; m3/m4 (parse diagnostics: unterminated `if`, stray `;`)
  identical modulo the $0 path prefix (invocation artifact, both shells
  print their own $0).
- configure prefix diagnostics: 6000-line and 9000-line truncations give
  GNU-identical rc=2 and error text ("unexpected end of file from
  `case'/`if' command on line N" — same line numbers).
- Grouping stability: identical 106 group boundaries on the configure
  3000-line prefix before/after the incremental checkpoint.

### What remains (next lane / captain)

1. The two scanner false-positive families above (continuation.rs +
   the needs_more keyword stage) — they gate configure-full AND they
   are correctness bugs (grouping affects execution semantics in the
   grouped driver).
2. nvm -n is now 65.7x; the remaining ~1.6s is ~0.6s tokenize
   (tokenize_with_boundary over accumulated lines), ~0.4s the join-loop
   scans on `$`-dense lines (per-line O(line) after this round, but
   still re-run per join), ~0.6s nested body re-parses (each body runs
   the full parse loop; GNU parses each token once).
3. The hot-path -25% target needs the expansion/assignment internals
   (probe 15's `${a#..}` walker already has the perf3 fast path; the
   remaining work is the `$a` reference expansion and the `[`/`test`
   dispatch, distributed).

## perf4-continuation round (2026-09-28, wt8/perf4 rebased on e207b0df)

Fifth attack at #241/#242. Breakpoint disposition first: the predecessor's
uncommitted work had already landed as wt8/perf4 cd6d1fa7 (clean tree at
resume). Master had moved 25 commits (sourcefix grouped driver with the
parked `GroupScanFeeder`, perf5 join-gate re-land, ombperf assoc/arrays
memos), which superseded two of its eight parts wholesale — the tokenizer
join-gate dedup (identical fix re-landed on master as e3e539c6) and the
grouped-driver `ConstructScanCheckpoint` (master's feeder redesign solves
the same O(group^2) differently). The branch was rebased onto e207b0df
keeping the unique parts — Rc<str> source threading through the compound
parsers (the 272MB clone kill), the skip.rs `${`-arm zero-copy fix, the
env-stamp stack buffer, the `$(`-admission on the stray-`)` guard merged
with master's rubash#284 balancer gate — and dropping the two superseded
parts (lexer/mod.rs and script_driver.rs taken from master during the
rebase conflicts).

### Root cause this round: the history driver kept its own quadratic gather

`run_script_with_history_in` — the driver every `set -o posix` /
`expand_aliases` / history-bearing script takes (`script_uses_aliases`, so
configure and every autoconf-generated script) — still carried the OLD
inline gather loop: per appended physical line it ran
`expand_group_aliases(&pending)` (a whole-pending String copy even when the
table is empty) and `stdin_source_needs_more_posix(&expanded_pending)`,
whose token half TOKENIZES the whole accumulated group. configure-head1374
measured 13.3 MB of expansion input and 13.3 MB of needs-more input for a
38 KB script (106 groups, the largest 1050 lines) — O(group^2). Master's
feeder-based `read_next_source_group` existed but only the `source`
builtin used it: the history path never got it, and the feeder's own
alias-live arm (whole-pending fresh scan) is what remains there.

Fix (root cause: the duplicated gather, not a guard): ONE gather
implementation. `read_next_source_group` now also builds the per-line
`(text, is_heredoc_body)` vec the history driver records, and
`run_script_with_history_in` calls it; the inline loop is deleted.
GNU anchors: parse.y:3557 `read_token` is a streaming reader that never
re-tokenizes consumed input; builtins/evalfile.c reads command-by-command;
parse.y:3249 alias_expand_token applies to the token being read (never
retroactively — the alias table is frozen while a group is gathered, since
nothing executes between appends). Equivalence obligations documented in
the code: the frozen-table arm's feeder answer is byte-identical to the
old fresh scan (expansion is the identity there); the alias-live arm keeps
the exact fresh whole-pending scan.

### Numbers (debug, back-to-back A/B vs pristine master e207b0df, same load, medians of 3)

| probe | master | lane | delta |
|---|---:|---:|---:|
| 21-configure-head1374-n | 2478ms | **656ms** | **-74%** (GNU 8ms: 310x -> 82x) |
| 23-nvm-parse-n | 2395ms | **1890ms** | -21% (Rc threading from the rebase) |
| 24-nvm-load | 5769ms | **5315ms** | -8% |
| 17-parse-flat8000-n | 397ms | **303ms** | -24% |
| 16-parse-flat8000 | 2076ms | 2089ms | flat |
| 04-loop-true-builtin-x2000 | 731ms | 725ms | flat |
| 05-arith-x5000 | 1043ms | 1016ms | -3% |
| 15-expansion-x5000 | 3569ms | 3534ms | -1% |

configure prefix curve (warm one-shot): 3000L 3.7s; 6000L 16.4s; 9000L
43.3s (was TIMEOUT-shaped before; still super-linear — see leftovers).
One-shot timings on a freshly built exe are Defender-inflated (~1.1s
constant); harness medians above are the honest numbers.

### Verification

- `cargo build` 0 warnings; `cargo fmt --check` clean; lib 497/497;
  regression 26/26; `cargo check --tests` and `cargo check --release
  --tests` clean; continuation.rs untouched.
- GNU-diff matrices m1-m4 (`target/perf4/matrix{1..4}.sh`, both shells
  from script files): byte-identical modulo the $0 path prefix.
- NEW matrix5 (`target/perf4/matrix5.sh`): group-boundary identity probe —
  `set -o history` routes a script through the changed driver and the
  final `history` listing prints one entry per gathered group; covers
  multi-line quotes, heredocs (+ <<-), function bodies, case/if spans,
  trailing connectors, backslash continuations, LIVE aliases including the
  alias-value-opens-a-quote shape (`alias m5openq="echo 'opened"` closed
  by a later line) and multi-line alias values. Pristine master and this
  lane binary produce byte-identical stdout+stderr+history.
- Boundary identity on real corpus: instrumented master-ref build
  (throwaway worktree at e207b0df) vs lane, `GROUP start=... lines=...`
  dumps identical on configure-head1374 and the 3000-line prefix (106
  groups each).
- Known pre-existing (NOT from this change, master shows it too): the
  joined history echo prints `;;;`/`&&;` where GNU prints `;;`/`&&`
  (history-join display, separate from grouping).

### Leftovers (honest)

1. **configure-full -n still TIMEOUT (>120s).** The remaining quadratic
   is the TEXT half of the completeness test at candidate lines
   (`stdin_source_text_needs_more`): 615 calls / 4.6 MB scanned on
   head1374; per-probe true-answer split: syntax family (quotes/comsub/
   array/close-char) 315, unclosed-function-body 188, signature 4,
   continuation 3. A checkpoint cannot skip these — the group is open
   BECAUSE a construct is open, so the scanner state at each line boundary
   is legitimately non-initial; the fix is state-carry (resumable)
   versions of the four text scanners, whose quote/comsub state machines
   live in src/lexer/continuation.rs (captain-exclusive, #292). The
   function-body probe is in script_driver.rs (this lane's file) and is
   the next attackable piece (~188/615 of the calls), but its incremental
   form must reproduce first_unquoted_function_body_delimiter +
   unquoted_delimiter_depth semantics exactly — re-implementing them in
   parallel risks a semantics fork (no-whack-a-mole rule), so it needs the
   same resumable-scanner discipline, not a copy.
2. Hot-path -25% (#242c) still NOT met: 04 flat, 05 -3%, 15 -1% vs
   master. The remaining cost is the distributed expansion/assignment
   machinery perffix/perf3 characterized (probe 15 is 3534ms vs GNU 57ms).
3. nvm-parse-n at 1890ms is 105x GNU (GNU 18ms this round; 25ms at the
   perf4 round — drvfs WSL timing varies); the <80x goal from #241 holds
   only on GNU-slower days. Remaining time: join-loop scans on $-dense
   lines and nested body re-parses (both characterized in the perf4
   round's leftover list).
## perf6 round (2026-09-28, wt10/perf6 on e207b0df): the two ombperf gaps

Goal: attack the two hotspots the ombperf round left open — the
pre-expansion validation double-walk and the per-command machinery floor.
All numbers release build, same-session base-vs-current (base binary =
e207b0df built in a detached worktree), median of 5, Windows subprocess
wall (python time.perf_counter) unless noted. GNU anchors = WSL 5.3.0
(/usr/local/bin/bash), inner $EPOCHREALTIME timing, script files under
target/perf6/.

### GNU anchors (this session, 20000 iterations, inner ms)

- null `for ((;;)) :` 40.3 (2.0 us/op) | local3 113.9 (5.7 us/op) |
  arridx `a[1]=$i` 39.7 (2.0 us/op)

### Gap 1 decision: KEEP the pre-scan, add a provable admission gate

Decision rationale (delete was considered first):

- Introduced by 341321cd (2026-06-22, "feat: support parameter expansion
  errors") as the ONLY detector for `${x?}`-family errors; since then ~12
  more error classes accreted into parameter_errors.rs.
- The real expansion path does NOT duplicate these diagnostics — it relies
  on the pre-scan. src/executor/expand_braced_special.rs (indirect base
  resolution): "an unresolvable base was already reported as `invalid
  indirect expansion` by the word error scan and expands empty here."
  Messages `cannot assign in this way` / `invalid indirect expansion` /
  `invalid variable name` / `substring expression < 0` exist ONLY in
  parameter_errors.rs. Deleting the pre-scan loses error detection unless
  all arms move into the expansion walker — a deep subsystem refactor
  (per-arm lazy-vs-eager semantics: expand_word.rs already notes bad
  substitution must fire only when a nested word is actually evaluated,
  e.g. `${x:-${(M)y}}` with x set stays silent). > 1 round; not attempted.
- Short-circuit instead: every arm of BOTH scans
  (parameter_assignment_error_in_word, parameter_expansion_error_in_
  word_context) requires an ASCII `$` in the word — `${` spans, or a
  `$`-introduced parameter reference in the nounset arm (the DATA_DOLLAR
  literal-`$` marker byte is skipped there and never forms `${`). A word
  without `$` provably returns None, so one memchr (`contains('$')`)
  gates the whole per-byte quote state machine. This is a necessary-
  condition admission, not a blacklist symptom guard; `$`-bearing words
  keep the full scan.

GNU citation for the eventual full delete: GNU has no pre-scan at all —
subst.c:11229 expand_word_internal expands once and reports inline
(parameter_brace_expand_error at subst.c:8221, expand_wdesc_error /
expand_param_error mapping at subst.c:4288-4296).

### Gap 1 error-path matrix (script-file probes, WSL 5.3.0 vs rubash)

15/15 byte-identical (stdout+stderr+rc, invocation-path prefix
normalized; artifacts target/perf6/errmat/ + run-errmat.py):
`${x?}` fatal rc1; `${x:?}` set-null; `${1=x}` cannot-assign;
`${@=x}` cannot-assign; `${ro=x}` readonly; `${!bad!}` bad substitution
(non-fatal, next line runs); `${x@C}` bad substitution FATAL rc1;
`${#:}` bad substitution; `${!unsetname}` invalid indirect expansion;
`${!x}` invalid variable name (value-dependent); `${a[@]:0:-1}`
substring expression < 0; nounset `$missing` unbound variable rc1;
`${x` unmatched `}` rc2; 2 dollar-free controls. Plus `bash -uc`-shape
fatal cases: `${x?}` and nounset `$missing` both rc=127 with identical
messages (shell-name prefix = each shell's own $0).

### Gap 2 decomposition (scratch phase timers + clone counters, removed)

Instrumentation: P_VALIDATE timer, word/dollar counters on both pre-scans,
clone counters at 5 candidate sites (arith cmd.clone, pre-alias
words.clone, strip early cmd.clone, expand field clones, loop redirect
clone), 4 sub-timers inside expand_command_words. All removed before
commit; numbers from RUBASH_EXEC_PROFILE=1 release runs.

- p6-arr (`a[1]=$i`, 20000 cmds): expand phase 2586 ms of which
  **glob/materialize block = 2412 ms** (120 us/cmd); the actual per-word
  expansion was 110 ms; premeta 55 ms. The ombperf hypothesis
  ("CommandNode clone/borrow missing") is refuted for this shape: full
  CommandNode clones measured only 20000/probe (c_strip) with ~190 KB
  total word bytes.
- Clone census per probe (count / word-bytes): c_strip 20000 / 190 KB
  (waste — nothing stripped), c_prealias 20000 (1-word Vec clone),
  c_arith 0, c_loop 0. Top 3 clone points by count: strip early-return,
  pre-alias words snapshot, expand_command_words field set (1/cmd).
- p6-null (`:`): for_test 198 + for_update 266 ms of 742 total (arith
  machinery, not the `:` command); validate 2.5 ms; expand 31 ms.
- OMB load: validate phase 264 ms (ombperf, pre-gate) -> 2.9 ms
  (post-gate, instrumented build; cross-session, machine load differs —
  the same-session base-vs-cur load delta below is the honest number).

### Landed changes (4 files; instrumentation fully removed)

1. **perf(executor): array-element assignment words never pathname-expand**
   (command_prepare.rs, expand_command_words suppress_glob). GNU
   general.c:477 assignment() accepts `name[sub]=value` (skipsubscript
   -> `]` -> `=`), parse.y:5785-5791 marks it W_ASSIGNMENT at parse, and
   subst.c:12476 separate_out_assignments() peels the leading
   W_ASSIGNMENT run BEFORE expand_words — assignment words never reach
   glob(). Rubash's parser keeps them in cmd.words, so pathname_expand_
   word globbed them: `a[1]=5` with a file `a1=5` in cwd ran
   `a1=5: command not found` (GNU 5.3.0 probe: assigns a=([1]="5")).
   Both a perf win (no directory scan per element write) and a GNU-
   alignment fix. Admission = parser's array_element_assignments marker
   (leading-run scope, mirrors separate_out_assignments); argument
   position (`echo a[1]=5` -> `a1=5`) keeps globbing in both shells.
2. **perf(executor): borrow instead of clone when nothing to strip**
   (command_execute.rs, strip_invalid_env_assignment_prefixes ->
   Cow<CommandNode>). The early return cloned the whole 55-field
   CommandNode for every command (20000/probe measured); GNU
   execute_simple_command walks WORD_LIST by pointer (execute_cmd.c:
   4550+) — the nothing-to-strip case borrows, the copy exists only when
   words are actually removed.
3. **perf(expansion): `$`-byte admission on the parameter-error
   pre-scan** (parameter_errors.rs, both scan entry points). See Gap 1.
4. **chore(test): drop unused `use super::*`** in prompt_command.rs
   tests (pre-existing master break of `RUSTFLAGS='-D warnings' cargo
   test --workspace --no-run`; no lane owns the file).

### Numbers (same-session base e207b0df vs perf6 HEAD, median of 5)

| probe (20000 iters) | base ms | cur ms | delta | GNU ms | ratio base->cur |
|---|---:|---:|---:|---:|---:|
| p6-null `:`          | 661.4 | 633.5 | -4.2%  | 40.3  | 16.0x -> 15.3x |
| p6-local3            | 4381.7| 3971.3| -9.4%  | 113.9 | 38.4x -> 34.7x |
| p6-arr `a[1]=$i`     | 3149.4| 1074.2| -65.9% | 39.7  | 78.8x -> 26.6x |
| OMB load (real HOME) | 1021.1| 973.8 | -4.6%  | —     | (earlier session: 1053.5 -> 956.8, -9.2%) |

Per-op (inner, minus ~20 ms startup): `:` 32 -> 30.5 us (GNU 2.0);
local3 218 -> 198 us (GNU 5.7); arridx 156 -> 53 us (GNU 2.0).

### Semantics gate

PS1 md5, `declare -p` md5 (_omb_spectrum_fg/FX/FG composite),
`declare -F` sorted md5 + count, `alias` sorted md5 + count: byte-
identical base vs current on the real-HOME OMB load (284 functions,
33 aliases; ombperf's 281/33 delta = OMB tree drift, same on both
binaries). Error-path matrix 15/15 byte-identical. cargo test --lib
497/497; --test regression 26/26; RUSTFLAGS='-D warnings' cargo test
--workspace --no-run clean; cargo fmt clean.

### Remaining hotspots (measured, for the next round)

- `for ((...))` test+update: 23 us/iter pair (465 of 742 ms in p6-null).
  eval_arithmetic_command_value does per-eval work even for constant
  expressions: arith_dynamic_values() HashMap build, expression
  .to_string() snapshot, ARITH_WRITES clear. GNU re-parses per
  iteration too (execute_cmd.c:3201 evalexp) but at ~1 us; a
  constant-expression memo is the candidate. One root cause per round.
- local3 198 us vs GNU 5.7: dispatch/matcmd machinery (matcmd 37 us/cmd
  debug-profiled) + `local` declaration-family path - the borrow-based
  CommandNode expansion refactor (deep subsystem, ombperf's original
  hypothesis) remains open; this round's clone census shows the biggest
  single clone (strip) already gone, next is the expanded.words snapshot
  and the expand field set.
- OMB load: `chain` phase (alias/time/if/pipe matcher per command)
  1393 ms of 8586 ms instrumented total - attribution only, not yet
  decomposed.

