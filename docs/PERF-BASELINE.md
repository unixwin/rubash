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


## perf7 round (2026-09-29, wt12/perf7 on 60418802): nvm.sh special project

Owner-named attack on the two worst rows: 23-nvm-parse-n (852x baseline,
105x after perf4) and 24-nvm-load (379x baseline). Base 60418802 already
contains perf4's gather unification (ae7f60f5), contperf's comsub residual
park/resume (a1f43085) and perf6. All numbers debug build; harness medians
of 3 PLUS back-to-back A/B against a pristine-base binary built the same
session (../rubash-wt-perf7-base at 60418802, alternating runs, same load)
— the GNU column varies with WSL/drvfs load per round, so rubash-ms to
rubash-ms is the honest comparison.

### This lane's own baseline (morning, harness)

| probe | rubash ms | GNU ms | ratio |
|---|---:|---:|---:|
| 23-nvm-parse-n | 2204 | 17 | 129.6x |
| 24-nvm-load | 5223 | 36 | 145.1x |

### Decomposition (temporary phase timers/counters, fully removed)

nvm -n's 2204 ms: **tokenize 1220** (lextok 695 — of which 1.8 MB of the
2.2 MB scanned came from 194 `}`-byte-refused full re-lexes, 107 of them
>= 8 KB buffers; compound scan 185; param scan 137; huq 112; rotate 20;
comsub advance 15) + **parse 860** (six fold passes 468; per-token balancer
10; 1577 nested body re-parses cloning **259.5 MB** of source text) +
unclosed-syntax prescan 26 + process floor ~130.

nvm-load's 5223 ms: exec profile 405 commands, matcmd 4380 ms dominated by
the single `. nvm.sh` source command: gather feeder ~1000 + the per-group
one-shot re-tokenize (see leftovers) + group parse + nvm's own commands.

### Landed changes (15 files; instrumentation removed before commit)

1. **`Rc<str>` source threading** (parse_loop.rs ParseLoopOptions/
   ParseState + if/loop/for/case/brace/subshell/function parsers +
   script_driver/source/execution/function_calls/job_builtins/trap_exec
   construction sites). `parse_body_with_diagnostics` cloned the WHOLE
   script text per nested compound-body re-parse (259.5 MB measured on
   nvm -n, ~1500 bodies). GNU anchor: parse.y keeps ONE input string per
   parse (shell_input_line family; yyerror slices it, print_offending_line
   parse.y:6813-6824). Re-land of the wt8/perf4 fix that was lost in that
   lane's rebase (master's ae7f60f5 carried only the gather unification).
   clone counter: 259.5 MB -> 0.
2. **Fold-pass fast exits** (parse_loop.rs): each of the six whole-list
   folds (pipeline/time-pipeline/time-simple/inverted/and-or/background)
   is the identity for lists lacking its trigger field (`pipe`/
   first-word-`time`/`inverted`/`and_or`/`background`), so a list-level
   `any()` skips the rebuild. 468 -> 31 ms. GNU anchor: the folds model
   grammar productions the token stream proved absent (parse.y:1337ff).
3. **Compound-assignment `=(` admission** (lexer/mod.rs join loop):
   `has_unclosed_compound_assignment` can only return true when a word
   ending `=` is immediately followed by `(` (continuation.rs
   opens_compound arm) — the adjacent byte pair admits the scan. GNU
   anchor: parse.y:5785-5791 recognizes the compound assignment from
   exactly this adjacency at token-read time. 185 -> 17 ms (nvm.sh has
   zero `=(` lines).
4. **Param-expansion `$` admission + false-answer cache** (`param_open_cache`,
   lexer/mod.rs): the scan's only true exit is a `${` whose body scan
   failed, so a `$`-free line admits false and a cached false survives a
   `$`-free append; same invalidation set as `unclosed_quotes_cache`
   (IFS_GLUE insert, backslash pop, rotation, commit). 137 -> 86 ms.
5. **skip.rs `${`-arm zero-copy + balancer admission**:
   `command_substitutions_balanced`'s per-`${` String copy of the entire
   remaining input replaced with the `scan_braced_parameter_body_chars`
   slice API (rubash#185; identical shape to the captain's landed
   continuation.rs fix — slice INCLUDES `${`, `index += 2 + scan.end`);
   the parse loop's per-word-token balancer consult is admitted on
   `$`/backtick presence (all three false-exits of the balancer open with
   an unquoted `$` or a backtick). 10 ms on nvm (bigger for the
   configure/m4sh family per wt8 data).
6. **Brace-fold-aware boundary resume gate** (lexer/mod.rs): the blanket
   "appended line contains `}` -> refuse resume" (rubash#281 safety gate)
   is replaced by the exact condition it stood in for: resume is refused
   only when a pre-checkpoint `{`-arm fold can actually complete — i.e.
   the checkpoint tokens contain a bare `{` Keyword AND the innermost
   one's `skip_brace` (resumed from the shared BraceScanCache
   continuation, O(appended tail)) reports `closed` on the current buffer.
   No bare `{` -> no pre-offset opener exists -> resume; not closed (the
   `}` bytes are quoted/word-glued/nested-open) -> braces nest, so no
   outer group can close either -> resume. Soundness rests on the token
   `}`-arm and carried parse state being checkpoint-identical (same
   induction as the existing resume) and on `skip_brace` being the same
   pure scan the fold runs. GNU anchor: parse.y:3557 read_token reads the
   input exactly once; 194 refused passes re-lexed 1.8 MB (570 ms) though
   the fold completes on only a fraction. refused 194 -> 40, scanned
   2.2 -> 0.5 MB, >= 8 KB passes 107 -> 13.

### Numbers

| probe | base (morning) | perf7 harness | A/B medians (same load) | GNU this round |
|---|---:|---:|---:|---:|
| 23-nvm-parse-n | 2204ms / 129.6x | **1375ms / 55.0x** | 2263 -> 1337 (**-41%**) | 25ms (17 morning) |
| 24-nvm-load | 5223ms / 145.1x | **3564ms / 91.4x** | 5223 -> 3564 (**-32%**) | 39ms |
| 17-parse-flat8000-n | 390 (perf3 day) | 379ms | 82 -> 77 ms (-6%) | — |
| 21-configure-head1374-n | 2478 (master, perf4c) | 743ms | 78 -> 73 ms (-6%, warm one-shot) | 9ms |
| 16/15/04 | — | — | +3% / +1.8% / +2.9% (noise band, spreads overlap) | — |

configure-full -n: still TIMEOUT at 125 s on BOTH binaries (blocked on the
captain-family scanner false positives documented in the perf4 round; this
lane's gates do not address them).

### Semantics gate (zero-change evidence)

- nvm `--no-use` load on pristine-base vs lane binary: `declare -p` of
  every nvm_* variable, `declare -F`, `compgen -A function`, `type nvm`,
  `nvm --version`, `alias` listing — byte-identical; the ONLY diffs are
  PPID (per-process) and each binary's own BASH/THIS_SH path.
- Brace-resume matrices (target/issue-suites/results/perf7/matrix{1,2}.sh):
  multi-line function bodies with `}` bytes in strings/comments/params,
  glued `}}` closers (rubash#278), nested same-line folds, heredocs in
  groups, backslash continuations, comsub-in-group — base binary and lane
  binary byte-identical stdout+stderr+rc; GNU parity modulo the $0 path
  prefix and $HOME-derived test data (matrix1's `${HOME##*/}`).
- cargo test --lib 505/505; --test regression 27/27; RUSTFLAGS='-D
  warnings' cargo test --workspace --no-run clean; cargo fmt --check
  clean; cargo check --tests and --release --tests clean;
  src/lexer/continuation.rs untouched.

### Acceptance (honest)

The <10x / <200 ms target for nvm-parse is NOT met: 129.6x -> 55.0x
(harness ratio, GNU 25 ms this round; at the morning's GNU 17 ms the same
1375 ms is 81x). Wall: -41%. nvm-load 145.1x -> 91.4x (-32% wall). The
remaining costs are characterized below; the biggest two live outside this
lane's sanctioned files or need a deep-subsystem pass.

### Leftovers (measured, with owners)

1. **huq 112 ms (nvm -n) / 336 ms (load)**: `has_unclosed_quotes`
   re-scans the whole accumulated logical line per non-quote-inert append
   (16009 calls on load). The incremental park/resume state machine lives
   in src/lexer/continuation.rs — captain-exclusive (same shape as the
   landed comsub residual checkpoint, rubash#292). Hand to captain/perf8.
2. **Source double-tokenize (~1 s of nvm-load)**: the `.` builtin's
   gather (read_next_source_group) tokenizes each group incrementally,
   then the group's parse re-feeds the same text through
   tokenize_with_heredocs' join loop (huq calls 16009 vs 5764 on -n).
   Root fix = return the feeder's tokens from read_next_source_group and
   use them when the group text was not alias-rewritten. Equivalence
   obligations before landing: mid-group posix/extglob flips (the one-shot
   re-lex uses the group-END mode while the feeder tracked per-line - a
   flipping group lexes differently today and the reuse must refuse it),
   Token::column semantics (byte offset into the logical line vs byte
   offset into the group text plus the current `column += line_offset`
   arithmetic), and the alias-live arm keeping the fresh scan. This lane
   did not attempt it (rubash#117 discipline: a wrong invariant here is a
   silent semantic bug, cf. the worked counter-example).
3. **Parse main loop ~390 ms** (after folds 31 ms): distributed per-token
   costs in handle_token's compound-parser chain; no single >5% site found.
4. **param scan 86 ms**: `$`-bearing appends inside open spans re-scan
   the whole buffer; needs a resumable scanner in brace_scan.rs (same
   discipline as rubash#292; not attempted this round).
5. Startup floor ~130 ms (probe 01 family, 13x) - unchanged, separate
   lane (#242).


## perf10 round (2026-09-29, wt13/perf10 on 116b12aa): source double-tokenize + startup floor

Two perf7 handoffs (leftovers 2 and 5). Base 116b12aa carries perf7/perf8
and the rubash#295 carrier fix. All numbers debug build. Sibling lanes
(perf9/ecosweep2/fixpack) were active on this host during the round: the
harness's absolute numbers moved with their load (probe 16 executed-shape
380 -> ~2140 ms on BOTH base and lane binary), so every accept/reject
decision below rests on back-to-back alternating A/B against a pristine
base binary built this session (target/rubash-base-perf10.exe), same load,
plus same-morning harness anchors.

### Task 1: source double-tokenize — LANDED

Root cause (perf7's measurement): `read_next_source_group`'s gather feeds
each physical line to a parked `GroupScanFeeder` (completeness state +
token stream), then `run_source_groups` re-fed the same joined group text
through `tokenize_with_initial_posix` — a full second scan of every byte
(huq calls 16009 on nvm-load vs 5764 parse-only).

The enabling fact: `tokenize_with_heredocs` IS a fresh `GroupScanFeeder`
plus one `push_line` per physical line plus `finish()` — the batch
tokenizer and the gather feeder are the same code. The reuse returns the
feeder's committed tokens instead of re-lexing, gated on the three perf7
equivalence obligations, each argued against GNU:

1. **Mid-group posix/extglob flips** — posix is per-feeder state
   (`parse_posix` flipped per logical line at lexer/mod.rs
   `line_posix_mode_change`); the gather feeder and a fresh re-lex start
   from the same executor mode (nothing executes between the gather's
   capture and the caller's parse in `run_source_groups`) and replay flips
   at identical lines -> identical streams. SAFE. extglob is a
   process-Global (`PARSE_EXTENDED_GLOB`): the re-lex replays shopt flips
   from the group-END value, so its pre-flip lines can see a different
   gate than the streaming feed did (parse.y:5466 gates `?(`/`*(`/...
   pattern chars per token in read_token_word; tokens snapshot the gate —
   `Token::extglob_gate`). NOT provably equivalent -> the feeder now
   tracks `extglob_toggled` (an EFFECTIVE Enable/Disable toggle) and the
   gather refuses reuse when set, falling back to today's fresh re-lex
   byte for byte. GNU anchors: parse.y:3248-3249 alias_expand_token,
   builtins/evalstring.c:315 parse_and_execute (read one command, execute,
   read the next — mode changes take effect between commands, never
   retroactively).
2. **Token::column/position semantics** — identical by construction:
   committed tokens in both streams carry `position = logical_start_line`
   (physical line within the group, both start at line 1) and `column` =
   byte offset into the logical line; `leading_ws` capture and
   heredoc-end-line arithmetic are the same code. The reuse applies the
   caller's unchanged `position/column += line_offset` post-pass. The
   feeder's trailing line-break separator is popped with the exact rule
   `tokenize_comsub_body_with_origin` applies to the fresh stream.
3. **Alias-live arm** — untouched: the gather only builds a feeder when
   the alias table cannot rewrite text (`!enabled || table empty`), which
   is exactly when `expand_group_aliases` is the identity (exec text ==
   pending). With live aliases there is no feeder and the caller keeps its
   fresh re-lex of the rewritten text (parse.y:3249 alias_expand_token
   fires per token while READING).

A fourth construction-level condition is documented in the code: the
group must have broken COMPLETE (an EOF-exhausted incomplete group takes
`run_source_with_line_offset`'s unclosed-diagnostics re-lex), and the
push-sequence identity (batch `split('\n')` final-empty-piece skip vs the
gather's committed break point) holds because remaining pieces belong to
later groups.

**Differential evidence** (new `perf10_token_reuse_tests`, 9 tests): the
real gather driven over plain/function/brace-group/heredoc/continuation/
comsub/CRLF/blank-line/posix-flip corpora AND the full vendored nvm.sh
(172906 bytes, the load probe's exact fixture), asserting every Token
field byte-identical between the reused stream and a fresh re-lex of the
same group bytes, with production's gather-then-parse interleaving; plus
an assertion that a `shopt -s extglob` group REFUSES reuse.

| probe | base (morning harness) | lane harness | A/B medians (same load) | GNU this round |
|---|---:|---:|---:|---:|
| 24-nvm-load | 3154 ms / 92.8x | **2475 ms / 72.8x** | 3190 -> 2458 (**-23%**) | 34 ms |
| 23-nvm-parse-n | 1202 ms / 75.1x | 1196 ms (unchanged, expected) | — | 16 ms |
| 01-startup-empty | 68 ms | unchanged (A/B medians 53/54 ms) | — | 5 ms |
| 16-parse-flat8000 | — | — | 2141 -> 2149 (+0.4%, noise) | — |

**Semantics gate (zero change):** nvm `--no-use` load on pristine-base vs
lane binary: nvm_* `declare -p`, `declare -F`, `compgen -A function`,
`type nvm`, `nvm --version`, `alias` listing, PS1 — byte-identical (7/7).
OMB live load (real HOME, agnoster, 252 functions / 33 aliases):
`_omb_spectrum_fg` md5, PS1 md5, function/alias counts — byte-identical.
83-suite spot checks (scripts/true-baseline.sh, WSL GNU 5.3.0 oracle):
dstack byte-parity 0 diff; quote (94/62/4 diff lines) and comsub
(27/15/0) rubash outputs byte-IDENTICAL base-vs-lane (pre-existing master
diffs, counts unchanged). cargo test --lib 520/520; --test regression
27/27; RUSTFLAGS='-D warnings' cargo test --workspace --no-run clean;
cargo fmt --check clean; cargo check --tests and --release --tests clean;
src/lexer/continuation.rs untouched.

Files: src/lexer/mod.rs (extglob_toggled flag + getter), src/script_driver.rs
(read_next_source_group returns feeder tokens; differential tests),
src/builtins/source/execution.rs (consume reused tokens).

### Task 2: startup floor — decomposed, no rubash-side target >= 20 ms exists

Probe 01 (68 ms from the Git-Bash harness, GNU 5 ms inside WSL) decomposed
with temporary GetProcessTimes + phase instrumentation (removed):

| component | warm ms | note |
|---|---:|---|
| MSYS-parent spawn overhead | ~54 | `cmd /c exit` measures 53 ms from the same parent — NOT rubash's; a native parent (PowerShell) spawns rubash in 13.9 ms vs cmd 13.7 ms |
| process creation -> main entry | 7.5-8.8 (cold 173-227) | loader + CRT + main-thread spawn of the 16.6 MB debug image; 512 MB stack reserve ruled out by A/B (8 MB: identical) |
| locale init | 0.1 | |
| Executor::new total | 3.2 | env vars collect 0.24 + snapshot clone 0.05 + PATH import & POSIX-tools-dir fs probe 0.6 + fresh-shell-env (current_dir/current_exe/THIS_SH is_file/OLDPWD is_dir/var-tmp create) 1.0 + signal-mailbox create_dir_all+write 0.5 + struct+VariableStore::from_environment 0.3 |
| first parse pipeline (`exit 0`) | 1.2 | debug-build codegen; no lazy regex/table init exists (grepped: none in lexer/parser) |
| teardown + CRT exit | 1.7 | |

Rubash's whole in-process floor is ~14 ms (debug build) — same order as
GNU bash's total 5 ms in WSL, and within 0.2 ms of cmd.exe's floor from a
native parent. The 13.6x probe ratio is ~80% MSYS-parent spawn overhead
that cmd.exe pays identically. The perf7 expectation ("env large copy /
Vec prealloc / first lazy init") is falsified by measurement: the env
triple copy costs 0.35 ms total, and there is no lazy-init site. The
largest rubash-owned items are fs probes (~1.6 ms across tools-dir,
THIS_SH/OLDPWD stats, signal-mailbox, var-tmp) each of which is semantic
Windows-port work (command -p toolset pinning, cross-process kill
mailbox, /var/tmp fixture) — shaving them is sub-ms against a 54 ms
parent-side constant. No change landed; the honest floor owner is the
parent spawn path and the debug image, not Executor startup.

## perf11 round (2026-09-29, wt14/perf11 on 61069619): the per-command floor

Owner target "性能必须对齐 GNU": attack the remaining PERF-BASELINE big
rows item by item (per-command `:` floor, local3, `for ((...))` arith
pair, configure -n, nvm, OMB). All wall numbers RELEASE build, median of
7 (3 for configure-full), back-to-back alternating A/B against a pristine
base binary built this session from 61069619 (same load); GNU anchors
re-measured inside WSL this session (script files under `target/perf11/`,
copied from the perf6 fixture). Scratch instrumentation (exec_profile
phase extensions + a script_driver `perf11_scan` battery) fully removed
before commit.

### Full-suite panorama at round start (debug, scripts/run-perf-suite.sh)

22-configure-full-n now COMPLETES at 15.0s (was TIMEOUT >120s before
perf8/9; GNU 37ms → 406x) — the top row by ratio. 23-nvm-parse-n
71.8x, 24-nvm-load 70.2x, 16-parse-flat8000 131.8x, 19 168.8x (parse
canary), 04/05 53-85x, 15 61.8x. Spawn parity (08 1.5x, 09 0.9x)
retained. Release-side gap list (owner table): cfg-full-n ~3.66s,
nvm-n ~232ms, nvm-load ~454ms.

### Decomposition (scratch timers; nested-inclusive sums, removed)

p6-null `for ((i=0;i<20000;i++)); do :; done` (base 663ms release): the
`:` command itself costs ~3.5 us (expand 31ms + matcmd 13ms + linecmd
22ms per 20000) — **65% was the for-arithmetic pair: test 204ms + update
272ms = 23.8 us/iter**, of which arith_dyn (the 9-entry dynamic-parameter
HashMap built per evaluation) 182ms, the parse/eval core 247ms.

p6-local3 `f() { local a=$1 b=$2 c=$3; }` (base 3543ms): per call the
`local` command's execute_local was 72 us, of which the declare core only
13.6 us — **the surrounding frame machinery 57 us**, and inside it the
variable-attribute marker strings dominated: `is_marked_var` was called
1,540,024 times and `set_marked_var` 1,200,000 times (60 marker writes
per call) in 20000 iterations = 1701ms of the 3.5s wall (48%). The
per-eval `std::env::var("RUBASH_DEBUG_ARITH")` consult cost ~3 us per
arithmetic evaluation (Windows environ scan), i.e. ~45% of the whole
eval; a second one (`RUBASH_DEBUG_ASSIGN`) sat inside the per-assignment
loop.

configure -n (release 3.66s, gather 2967ms): 95% of the gather is
`text_scans.needs_more`'s comsub arm — **n_comsub 2812ms over only 3516
candidate-line calls = 554,841,139 chars walked (158K per call)**. The
parked `ComsubResidualState` parks at a `$(` whose
`skip_parenthesized_unit_ex` failed, and every later candidate line
re-derives from that park through the whole accumulated tail (the park is
semantically load-bearing: a longer buffer may let the atomic skip close
the unit and take a different trajectory than the char-by-char fallback,
so the checkpoint MUST re-derive to keep per-prefix answers identical to
the fresh scan). See leftovers — captain item.

### Landed changes (6 files; zero-semantic-change gates below)

1. **Marker-list no-op fast paths** (`env_helpers.rs unmark_env_name`,
   `read_split.rs mark_env_name`). GNU keeps attributes as flag bits on
   each SHELL_VAR (variables.h:124-133 att_exported etc.) — clearing an
   absent attribute is a bit test. The `$`-joined marker-string encoding
   expressed that as collect+retain+join+insert of an UNCHANGED string —
   60 such no-op rewrites per `local a=x b=y c=z` call
   (set_var_attrs walks all ten attribute lists), 1.2M per local3 run.
   A name that is not currently marked leaves the stored string
   byte-identical, so a zero-alloc membership pre-check (the identical
   split comparison `is_marked_var` uses) short-circuits both directions.
2. **SHELLOPTS/BASHOPTS read the maintained value**
   (`dynamic_arrays.rs dynamic_parameter_value`). GNU never re-renders on
   read: both are ordinary (readonly) variables whose VALUES are rebound
   at every option change (builtins/set.def set_option ->
   reset_option_vars for SHELLOPTS; builtins/shopt.def toggle_shopts ->
   set_bashopts for BASHOPTS), and every reader — parameter expansion or
   find_variable via expr.c:1150 expr_streval — returns the stored value.
   Rubash's flip sites maintain the stored entries the same way
   (set_shell_option / sync_shell_option_flag rewrite SHELLOPTS after
   every `__RUBASH_SETOPT_*` write — those two are the only production
   writers, verified by grep; shopt.rs's single SHOPT_STATE mutation site
   rewrites BASHOPTS). The read now takes the stored entry and renders
   only when absent (unit-test maps without Executor::new init). This
   removes two whole-option-table renders per arithmetic evaluation (the
   arith snapshot) — 63% of arith_dyn.
3. **Debug env knobs resolved once** (`arithmetic/mod.rs
   RUBASH_DEBUG_ARITH`, `temporary_assignments.rs RUBASH_DEBUG_ASSIGN`;
   the RUBASH_EXEC_PROFILE ensure_init precedent). GNU's expr.c debugging
   is compiled out entirely (EXPRDEBUG); a per-evaluation environ scan
   cost more than the parse itself on Windows (~3 us of ~6.6 us per
   evaluation). Note: the knob must now be present at process start.
4. **`eval_mutable_arith_result` no longer `into_owned()`s the normalized
   Cow** (`arithmetic/mod.rs`): a backslash-free expression (the common
   case — normalize returns Borrowed after a memchr) paid a full-string
   clone per evaluation; the parser now borrows the Cow directly (the
   error-record site materializes only on the error path).
5. **Line stamps insert only when changed** (`public_accessors.rs
   stamp_env_usize` + equality gate in set_current_command; re-lands the
   wt8/perf4 fix that was lost from master). GNU's line_number is a C int
   (execute_cmd.c SET_LINE_NUMBER) and the_printed_command a global —
   restamping per command is free there. Here every command stamped
   __RUBASH_CURRENT_LINE/__RUBASH_CMD_START_LINE twice (reader loop +
   execute_command) plus both command-text keys — a loop body re-inserts
   byte-identical values thousands of times; the equality gate is
   idempotence-preserving.

### Numbers (release, A/B vs pristine 61069619, same session/load)

| probe (20000 iters) | base ms | lane ms | delta | GNU ms | ratio |
|---|---:|---:|---:|---:|---:|
| p6-null `:`         | 663.1 | 393.7 | **-40.6%** | 40.6 | 16.3x -> **9.7x** |
| p6-local3           | 3542.5 | 2287.5 | **-35.4%** | 112.4 | 31.5x -> 20.3x |
| p6-arr `a[1]=$i`    | 1093.0 | 668.9 | **-38.8%** | 39.7 (perf6 day) | 27.5x -> 16.8x |
| 05-arith-x5000 (debug suite) | 1021 | 590 | -42% | 12 | 85.1x -> 49.2x |
| 06-strconcat (debug) | 1679 | 1446 | -14% | 23 | 73.0x -> 62.9x |
| 04-true-loop (debug) | 691 | 617 | -11% | 13 | 53.2x -> 47.5x |
| 15-expansion (debug) | 3522 | 3275 | -7% | 57 | 61.8x -> 57.5x |
| nvm-load (release)  | 454.0 | 442.6 | -2.5% | 35-39 | ~12-13x |
| nvm -n / configure -n | flat | flat | parse-bound | | |

The owner's per-line <10x target is MET for the null-command floor row
(9.7x release); local3 lands at 20.3x (see leftovers — the remaining cost
is the marker-string attribute model itself). OMB (real HOME, agnoster):
727 -> 693ms (-4.7%), gates below.

### Semantics gate (zero-change evidence)

- OMB live load, base vs lane binary: PS1 md5, `declare -p
  _omb_spectrum_fg`, `declare -F | md5sum`, `alias | md5sum` — 4/4
  byte-identical.
- GNU-diff matrices (script files, stdout+stderr+rc separately captured,
  `target/perf11/matrix{1,2,3}.sh`): matrix1 (SHELLOPTS/BASHOPTS after
  set -o/shopt flips in both directions, assignment/unset readonly
  rejections, $SHELLOPTS arithmetic-error text, `local SHELLOPTS`
  rejection, function-frame flip visibility) and matrix2 (local frames:
  -i attribute math, unset-after-return, outer-value preservation,
  nested frames, readonly local blocking, declare -p rendering) are
  byte-identical modulo each shell's own $0 prefix. matrix3 (arith
  dynamic vars BASHPID/BASH_SUBSHELL/FUNCNAME, loop + function LINENO,
  unbound-variable arith diagnostics, precedence/shift/mask/ternary
  math) fully identical. ONE divergence in matrix1 — GNU discards the
  whole `||` list after an arithmetic word-expansion error while rubash
  runs the fallback branch — is PRE-EXISTING (pristine 61069619 base
  binary reproduces it byte-for-byte; arith-error containment family,
  cf. rubash#306).
- cargo test --lib 537/537; --test regression 27/27; `RUSTFLAGS='-D
  warnings' cargo check --tests` and `--release --tests` clean; cargo
  fmt --check clean; src/lexer/continuation.rs untouched.

### Leftovers (measured, with owners)

1. **configure -n's comsub-park quadratic — CAPTAIN (continuation.rs)**
   2812ms of the 3.66s is the parked `ComsubResidualState` re-deriving
   from a stuck `$(` park on every candidate line (3516 calls, 554M
   chars, 158K/call avg). The park placement is semantically load-bearing
   (atomic-skip vs char-by-char trajectories can diverge), so the fix
   belongs to the #292 park/resume family: either prove the trajectories
   agree on the end-state `is_open()` answer (then commit incrementally
   and park only on genuine `esac)`-lookahead ambiguity), or fix the
   underlying `skip_parenthesized_unit_ex` divergences (the perf4-era
   as_fn_mkdir_p false-positive class) so parks stop sticking for
   hundreds of KB. Without this, configure -n cannot move; with it, the
   measured remainder is ~700ms (gather-feeder 100ms + parse 430ms +
   tokenize 90ms + prescan 26ms) ≈ 15-19x.
2. **Marker-string attribute model → attributes on the variable**
   (deep-subsystem ticket, next round): local3's remaining 114us/op is
   77 per-iteration `is_marked_var` checks (HashMap get + split of the
   marker string, ~517ms per run even after the no-op-write fix) plus
   the l_frame snapshot copies (21us). GNU stores att_* bits on the
   SHELL_VAR; the refactor keeps the marker strings only as the
   cross-process export encoding.
3. **nvm -n 232ms (12-14x) / nvm-load 443ms**: parse-bound (tokenize 90ms
   + parse 108ms on the batch path) — the perf7-characterized
   distributed parse-loop costs; no single >5% site.
4. **OMB chain phase**: 796ms (nested) of the load's profile after this
   round — the per-command reader-loop chain (dispatch-kind matchers,
   trap consults); linecmd inside it is now cheap. Next decomposition
   pass should sub-time the matcher chain.
5. Startup floor (probe 01, 15.6x): perf10's decomposition stands —
   ~80% MSYS-parent spawn constant, in-process floor ~14ms.

## exphot round (2026-09-29, wt15/exphot on 837514ff): the expansion main loop

Owner goal "all suites 2-3x": attack PERF-BASELINE 04/05 (debug 53-85x at
round start) and probe 15 (3534ms vs GNU 57ms) via the expansion hot path.
Base 837514ff carries perf6 (glob/materialize, `$`-admission pre-scan) and
perf11 (per-command floor round 2). All A/B numbers same-session alternating
runs against a pristine 837514ff binary (../rubash-wt-exphot-base), medians.

### Decomposition (scratch phase timers + counting allocator, removed)

Probe 15 debug (5867ms instrumented run) split as: **empty_rhs 1248ms**
(assignment-RHS expansion; 25000 RHS x 50µs, **75 allocations per `${a#pat}`
fragment**), empty_apply 350ms (apply_shell_assignment), empty_pre 167ms
(parameter-error pre-scan), expand 349ms (`[ "$i" -lt N ]` word pipeline,
23 allocs/word), chain 274, matcmd 205, linecmd 168, scans 101, jobs 92,
while-loop machinery ~240. Probe 05: arith 313ms = **arith_dyn 137 (the
per-eval dynamic-values HashMap build)** + arith_parse 137 + sync 7; loop
machinery ~90. Probe 04: test-command expand 155ms (60000 words; `[`/`]`/
`"$i"` ~13-26µs each), matcmd 84, empty 111.

Allocation attribution inside one `${a#alpha}` fragment (probe 15):
43 in the pattern arm — of which the bulk was **two `Vec<char>` stagings per
CANDIDATE BOUNDARY inside `case_pattern_matches`** (the `${a%%:*}` /
`${a%%gamma*}` removal loops call it once per boundary); ~12 in the
operator-dispatch chain (sequential `split_once_outside_subscript` probes,
each a full quote/bracket state-machine re-scan, `Vec<u8>` per `_str` op);
the rest in guards/marker chains/value fetches. The RHS walker tail paid
4-6 chained `str::replace` calls (each a full String copy even with ZERO
markers present), the assignment quote-removal tail 6 more.

### Landed changes (11 files; instrumentation removed)

1. **TopLevelOpIndex** (expand_braced_ops.rs + parameter_words.rs chain):
   GNU `parameter_brace_expand` (subst.c:9777) extracts the name up to the
   FIRST operator char in ONE `string_extract(..., "#%^,:-=?+/@}", SX_VARNAME)`
   pass (subst.c:9799-9802) and dispatches on that character
   (subst.c:9886-9917). The port ran ~8 sequential split probes per
   fragment, each re-scanning with the same state machine;
   `expand_quoted_parameter_word_mut` now builds one index per fragment
   (state machine copied verbatim from `split_once_outside_subscript_impl`;
   an operator byte at a top-level position never alters the machine's
   state, so pair/single answers are byte-identical). All 8 split sites in
   the chain now O(1) lookups; `split_once_outside_subscript_str` no longer
   allocates a `Vec<u8>` per call.
2. **simple_glob_matches** (conditional/pattern.rs): direct char-wise match
   with one saved backtrack point for the printable-ASCII literal + `*` +
   `?` class — the gnulib strmatch.c `gmatch` shape (no staging). The staged
   matcher (2 `Vec<char>` per call) was paid once per candidate boundary by
   the removal loops: pattern-arm allocations 43 -> 15 per fragment.
   nocase folds ASCII-only (`eq_ignore_ascii_case`, same as `chars_match`).
3. **arith_dynamic_values** snapshot: `HashMap<&'static str, String>` +
   `with_capacity(9)` (kills 9 per-name key clones + 4+ table growth
   reallocs per arithmetic evaluation). GNU has no snapshot (expr.c:1150
   expr_streval -> find_variable on maintained entries) — the map is port
   scaffolding; its build cost 137 -> 87ms per probe-05 run (instrumented).
4. **cow_replace + borrowing restore helpers** (markers.rs cow_replace /
   dequote_ctlesc_pairs_cow, parameter_ops.rs restore_cow,
   command_subst_helpers.rs unescape_cow; applied in the walker tail, the
   RHS hoist/restore and quote-removal tails): GNU restores markers IN
   PLACE (subst.c:4807 dequote_string / subst.c:4692 dequote_escapes walk
   once; a marker-free value is never copied) — every restore link now
   borrows when its sentinel is absent (output byte-identical).
   RHS-phase allocations 2.21M -> 565K per probe-15 run.

### Numbers (alternating A/B vs pristine 837514ff)

| probe | release base | release lane | delta | debug base | debug lane | delta |
|---|---:|---:|---:|---:|---:|---:|
| 15-expansion-x5000 | 612 | **530** | **-13%** | 3171 | 2942 | -7% |
| p15rel (4-assign x20000) | 2224 | **1943** | **-12%** | — | — | — |
| 05-arith-x5000 | 163 | 153 | -6% | 541 | 490 | -9% |
| p6arr `a[1]=$i` x20000 | 1258 | 1206 | -4% | — | — | — |
| p6-null `:` loop | 915 | 881 | -3% | — | — | — |
| 04-loop-true-builtin | — | — | — | 588 | 565 | -3% |
| 06-strconcat | — | — | — | 1414 | 1343 | -5% |

Release ratios (GNU anchors 57ms p15 / 13ms p05): probe 15 ~10.4x -> **9.3x**
(release), probe 05 ~12.5x -> ~11.8x. The 04/05-to-10x debug target is NOT
met (debug 04 ~44x, 05 ~38x): the remaining wall is distributed per-command
machinery (linecmd/chain/scans/jobs ≈ 40% of probe 04) and the RHS walker's
guard/memo/thread-local layer, not a single >5% site.

### Semantics gate (zero-change evidence)

- `cargo test --lib` 537/537; `--test regression` 27/27 (includes the
  golden issue296 carrier-adjacent-comsub fixture);
  issue315_guard_alternate 10/10; issue301_303_storage_literal 5/5;
  marker_leak_golden 6/6. `RUSTFLAGS='-D warnings' cargo check --tests` and
  `cargo check --release --tests` clean; cargo fmt clean;
  src/lexer/continuation.rs untouched.
- GNU-diff matrices under `target/exphot/matrix{1..4}.sh` (operator
  dispatch incl. error arms; pattern removal + literal/glob/nocase class;
  arith dynamic vars BASHPID/BASH_SUBSHELL/SHELLOPTS/BASHOPTS/PIPESTATUS/
  FUNCNAME/GROUPS + assignment marker restores + `${x@Q}`; set -u nounset
  interplay): **byte-identical base-vs-lane** (rc + stdout + stderr).
- 14 upstream suite slices (exp new-exp more-exp posixexp quote comsub
  comsub2 arith cond case errors varenv array assoc) run from
  third_party/bash/tests: **byte-identical base-vs-lane** (rc + stdout +
  stderr, binary-path prefix normalized).
- One rubash-vs-GNU divergence surfaced by matrix1 and verified
  PRE-EXISTING at 837514ff (base binary reproduces byte-for-byte):
  `set -u; echo "${u:=y} $u"` reports `u: unbound variable` while GNU
  continues — the nounset pre-scan does not model a preceding `${u:=y}`
  assignment in the SAME word (owner: parameter_errors.rs
  nounset_unbound_parameter; NOT from this lane's changes).

### Leftovers (measured, with the shape of the next fix)

1. **`"$i"`-class quoted-parameter words** (~24µs/word, probe 04/15 test
   commands): a fast path needs the exact GNU param_expand port for the
   single-name class — each special parameter (`@ * # ? - $ ! 0-9`) has
   distinct semantics and the class is NOT closed under a simple byte test;
   a hand-rolled admission here is exactly the rubash#117 counter-example
   shape. Next lane: mirror the walker's `$name` arm functions for an
   admitted class, skipping only provably-identity guard steps.
2. **Double pre-scan per RHS**: `apply_parameter_assignment_expansions_in_
   word` runs on the same text at assignment_expansion.rs (RHS entry) AND
   again inside the walker (embedded_mutations.rs:343). The second is
   memo-deduped (SubXpassFrame) but still a full quote-walk. Skippable by
   threading a "pre-applied" flag when `hoisted_value == value` (byte
   equality makes it provably redundant; subscript side effects are
   memo-keyed) — not attempted this round to keep the walker's parameter
   list untouched.
3. **arith_dyn 87ms remains** (probe 05): the 9-name snapshot still renders
   BASHPID/BASH_ARGV0/GROUPS/BASH_COMMAND/PIPESTATUS per evaluation. Root
   fix is GNU's model — maintained entries read via find_variable (the
   invalidation surface: subshell depth, function depth, per-command
   BASH_COMMAND stamp, option flips, pipeline status — wide; deep
   subsystem).
4. **empty_apply 350ms / empty_pre 167ms** (probe 15): assignment-store machinery
   (6 allocs/apply) and the command-level parameter-error pre-scan (perf6
   decided to KEEP the pre-scan for error ownership; GNU has none —
   subst.c expands once and reports inline — so the full fix is the
   perf6-deferred per-arm lazy-error refactor).
## perf15 round (2026-09-29, wt15/parsearch on 837514ff): parse-layer structural alignment

Owner target "下一波所有套件 2-3x 之内": structural GNU alignment of the
parse layer (tokenize+parse), the biggest remaining block of configure -n
(12-14x) and nvm -n (12-14x). All numbers RELEASE build, alternating A/B
against a pristine base binary built this session from 837514ff (same
load); GNU anchors re-measured inside WSL this session (nvm -n 18ms,
configure-head1374 -n 7ms). Scratch phase/alloc instrumentation (a
counting global allocator + phase timers) fully removed before commit.

### Decomposition (instrumented lane binary, nvm -n, 276ms base wall)

Whole-run totals at round start: **1,356,509 allocations / 251.6 MB** for
a 172 KB script; 2283 parse_with_options calls (1577 nested compound-body
re-parses), 15048 tokens, 7583 CommandNode::new. Struct sizes: Token 112
bytes, CommandNode **2240 bytes** (GNU make_cmd.h COMMAND is a ~32-byte
header whose union holds POINTERS to per-kind bodies). Phase ranking:

| # | parse-side cost | measured | owner |
|---|---|---:|---|
| 1 | Double per-word analysis: `push_command_word` ran 9 `record_*` scans, then `WordMetadata::new` re-ran the same scans (and re-PARSED every `$(...)` body a second time) into a duplicate store | word intake ~60ms/670K allocs nested-incl. | this lane |
| 2 | Feeder boundary re-lex (40 refused resumes re-lexing the whole accumulated line) | 45.3ms/211K allocs | #281/#292 captain family (untouched) |
| 3 | Join-gate param scan: `has_unclosed_parameter_expansion` re-collected the WHOLE logical line into a fresh `Vec<char>` per `$`-bearing appended line (nvm joins to one ~172KB logical line) | ~11ms + 18MB Vec<char> garbage (release memcpy is fast; debug far worse) | this lane |
| 4 | Fold clone chain: `fold_and_or_list`/`fold_pipeline` deep-cloned EVERY command (`commands[index].clone()`) whenever the fold ran (698 clones on nvm) | folds 13.9ms | this lane |
| 5 | Per-scan `chars().collect::<Vec<char>>()` even for scans that provably find nothing (plain identifiers) | ~9 allocs/word | this lane |

configure-head1374 cross-check: GATHER 11.8 / TOKENIZE 7.2 / PARSE 13.5ms
(grouped path); configure-full: GATHER 3170ms (the captain's comsub-park
quadratic — 86% of wall), PARSE 181.7ms.

### Landed changes (14 files; instrumentation removed before commit)

1. **Single-scan word intake** (`parser/support.rs push_command_word` +
   `parser/nodes.rs WordMetadata::from_scans`). Each analysis runs ONCE;
   the findings feed both stores — the CommandNode-level vectors (tagged
   clones, free for the empty Vecs that dominate real scripts) and the
   per-word WordMetadata (owning the scans' output). `from_scans` applies
   exactly `new`'s tagging (comsubs/procsubs carry word_index; the rest
   stay untagged), so both stores stay byte-identical to the double-scan
   output. GNU anchor: parse.y:5305 read_token_word assembles a word in
   ONE pass; make_cmd.c make_simple_command stores the WORD_DESC once —
   GNU has no per-word expansion scans at parse time at all. The eight
   now-dead `record_*_for_word` wrappers were removed.
2. **Param-scan incremental checkpoint** (`lexer/brace_scan.rs
   ParamScanState/param_residuals_advance` + `lexer/mod.rs
   ParamScanCheckpoint/advance_param_scan`). Same streaming model as the
   landed quotes/compound checkpoints (rubash#292 plan-B shape, perf8):
   per appended line the scan advances over the new tail from its
   snapshot; the single undecided position (a `${` whose body scan
   failed) parks and is re-derived, byte-identical to a fresh full scan
   by append-only induction. Invalidations mirror the sibling
   checkpoints (IFS_GLUE insert + rotation via rebuild_comsub_mirror,
   backslash-continuation pop, logical-line commit). GNU anchor:
   parse.y:3557 read_token streams input once; the whole-line
   recollect-and-rescan per appended line was this port's substitute.
   NOTE: the park/resume ADVANCE functions for quotes/comsub/compound
   live in continuation.rs (captain-exclusive) — this one lives in
   brace_scan.rs next to the scan it advances.
3. **Provably-empty scan admissions** (9 `_in_word`/`_in_raw` functions).
   Each scan's productions all require a trigger byte, so its absence
   proves the empty answer (rubash#117 whitelist discipline — class
   proof per scan, not a symptom blacklist): comsub needs `$`/backtick,
   arith `$`, param `$`, brace `{`, extglob `(`, tilde `~`, pathname one
   of `*?[`, word-quotes one of `'"\``, procsub `(`. Benefits the word
   intake AND all 101 standalone metadata sites (keywords, delimiters).
   GNU anchor: parse.y:5305 one-pass word assembly; GNU runs no per-word
   scans at parse time.
4. **Fold clone-chain → moves** (`parser/parse_loop.rs
   fold_and_or_list_commands`, `fold_pipeline_commands`).
   `std::mem::replace(&mut commands[index], CommandNode::new())` replaces
   `commands[index].clone()` — the index only ever advances and the taken
   slot is never re-read (skip-empty loops start from the current,
   untaken index), so the moved-out node reproduces the clone's result
   without the deep copy. GNU anchor: parse.y:1264 list / parse.y:1378
   pipeline productions LINK the already-built commands; make_cmd.c
   never copies a COMMAND.

### Numbers (release, A/B medians vs pristine 837514ff; GNU this session)

| probe | base ms | lane ms | delta | GNU ms | ratio |
|---|---:|---:|---:|---:|---:|
| nvm -n | 278 | 230 | **-17%** | 18 | 15.4x -> **12.8x** |
| nvm load (`--no-use`) | 486 | 427 | **-12%** | — | — |
| configure-head1374 -n | 87 | 80 | -8% | 7 | 12.4x -> 11.4x |
| configure-full -n | 3639 | 3364 | -7% | 37-46 (perf11) | parse share small vs 3.1s gather |
| 16-parse-flat8000-n | 47 | 46 | flat | — | per-line shape, no joins |

Instrumented phase deltas (nvm -n, same binary before/after each change):
PARSE 112.4 -> 68.7ms (**-39%**), TOKENIZE 96.7 -> 87.7ms (-9%), whole-run
allocations 1.357M -> 0.818M (**-40%**), fold passes 13.9 -> 1.9ms,
comsub parse fan-out 2283 -> 2131 parse calls (bodies parsed once, then
cloned). PARSE_SHARE of nvm -n wall (tokenize+parse phases): 209 -> 156ms.

### Semantics gate (zero-change evidence)

- nvm `--no-use` load, base vs lane (and vs the FINAL clean binary):
  declare -p of NVM_DIR/NVM_* env, `declare -F nvm_*` md5, compgen count,
  `nvm --version`, `type nvm` — byte-identical.
- OMB live load (real HOME, agnoster): PS1 md5, `declare -p` md5 of
  _omb_spectrum_fg/_omb_spectrum_f/FX/FG, `declare -F` md5 (284
  functions), `alias` md5 (33 aliases) — byte-identical.
- true-baseline spot checks (WSL GNU 5.3.0 oracle, scripts/true-baseline.sh):
  quote comsub dstack braces extglob arith case comsub-posix cond errors
  exp func heredoc read — rubash stdout+stderr byte-identical base vs
  lane for ALL 14 suites (pre-existing master diff counts unchanged:
  quote 85, comsub 25, dstack 0, braces 4, extglob 178, arith 0, case 3,
  comsub-posix 5, cond 12, errors 2, exp 264, func 0, heredoc 5, read 24).
- cargo test --lib 537/537; --test regression 27/27; RUSTFLAGS='-D
  warnings' cargo check --tests and --release --tests clean; cargo fmt
  --check clean; src/lexer/continuation.rs untouched (verified by diff).

### Leftovers (measured, with owners)

1. **Feeder boundary re-lex 45ms** (40 refused resumes × whole-line
   re-lex, 211K allocs/104MB) — the #281/#292 resume-refuse family,
   overlaps the captain's comsub-park work in continuation.rs. Not
   touched by this lane.
2. **Nested body re-parse fan-out** (1577 parse_body_with_diagnostics
   calls, ~150ms nested-inclusive): GNU parses compound bodies inline in
   the SAME reader pass (parse.y grammar recursion consumes body tokens
   directly); rubash re-runs the full parse pipeline per body slice. The
   deep fix is a single-pass recursive-descent body parse — a
   whole-parser architecture change, next-round scale.
3. **Word string triple-store** (token.value + cmd.words[i] +
   metadata.value/.raw — 3 copies per word; GNU stores each word string
   ONCE in the WORD_DESC): dedup needs WordMetadata field-type changes
   across 84 reader sites — assessed, deliberately deferred (churn/risk
   vs ~6ms estimate).
4. **CommandNode 2240 bytes** (18 Vecs + 5 inline Option<Redirect> +
   5 wrapper Options): measured impact on -n workloads is small (moves
   are rare after Change 4; the deep-clone costs are already gone);
   boxing the cold fields is a possible follow-up but not paying for
   itself now.
5. cfg-full -n remains gather-dominated (3.1s of 3.4s = the captain's
   park quadratic); with the park fixed, this round's parse-side cuts
   compound (~180ms parse + 107ms tokenize of the remainder).
## perf12 round (2026-09-29, wt15/execdeep on 837514ff): structured attributes

Deep-subsystem round attacking the perf11 leftover #2 (the marker-string
attribute model). Three commits on the lane branch, each gated with the
full battery. All wall numbers RELEASE build; the honest comparison is
the back-to-back alternating A/B against a pristine 837514ff binary
built this session in a detached worktree (same load) — the machine
drifted ~5% during the day, so single-shot "before/after" numbers from
different hours would lie. GNU anchors re-measured inside WSL this
session (script files under `target/perf15/`): local3 115.5ms, null
35.1ms, arr 43.0ms.

### Commit 1 — `VarTable`: the att_* port (a856bf29)

GNU stores every declare-family attribute as `att_*` flag bits on the
SHELL_VAR (variables.h:124-133). Rubash encoded them as membership in
`__RUBASH_*_VARS` DATA_DOLLAR-joined name lists inside the flat env
map — every query a split+scan, every write a collect+join+insert.

`ShellState.env_vars: HashMap` became `VarTable { values, attrs }`
(src/shell/var_table.rs): `attrs: HashMap<String, VarAttrs>` is the
structured att_* port, authoritative for the ten attribute keys; the
marker strings stay as the SERIALIZED form, synchronized on every real
bit flip (they remain load-bearing at the process boundary —
`Executor::new` imports a parent rubash's attribute lists from environ;
a fresh `${THIS_SH}` child rebuilds them via `child_shell_environment`
-> `from_values` — and for order-preserving list enumeration).
Deref/DerefMut/`IntoIterator for &VarTable` keep every value-only call
site compiling unchanged; the migration was compiler-guided up through
the builtin dispatch chain (declare/setattr/set/shopt/zsh/test/printf/
read/exec/complete/cd/arithmetic; value-only helpers like
`option_enabled` stay `&HashMap` and deref-coerce).

Reads: `is_marked_var`/`capture_var_attrs` (10 scans -> 1 lookup) and
44 builtin `marked_vars(...).contains(...)` sites -> `is_marked`.
Writes: every mark family funnels through VarTable methods that update
both forms; `set_attrs` rewrites a serialized list only when its bit
actually flips. Whole-list snapshot restores (tempenv
EXPORTED_VARS/NAMEREF_VARS) route through `restore_attr_string`.

Measured effect (decomposition, scratch battery removed): capture
162ms -> 3ms, set_var_attrs 229ms -> 7ms, builtin marked_vars
168ms -> (folded into is_marked ~63ms), per-run marker machinery
~865ms -> ~85ms instrumented. local3 2287 -> 1801ms.

### Commit 2 — process-env mirror gated to exported names (22a45701)

Decomposition found `set_process_env` running 60007x per local3 run
and 20007x per arr run — three Win32 SetEnvironmentVariable calls per
loop iteration for NOT-exported names:

1. for-loop variable write (word-list loop AND `i++`/`i=0` in
   `((;;))` through arith `set_variable`) — now routed through
   `sync_shell_assignment_process_env` (GNU execute_cmd.c:3201+ binds
   the loop variable through the ordinary assignment machinery; only
   exported/host-special names mirror).
2. `__RUBASH_CURRENT_LINE` stamp for needs-line commands — now skips
   the OS write when the value equals the last one WE wrote
   (`Executor::line_env_os_value` tracker). The gate must compare
   against our last OS write, NOT the env-map value: ambient-line
   restores write the map directly, and a map-equality gate left the
   OS env stale (alias diagnostics reported an earlier line — caught
   by the procenv gate).
3. Frame restore wrote every restored Some value to the OS env — now
   gated on the RESTORED attrs' exported bit (GNU pop_var_context
   reinstalls the saved SHELL_VAR, value + attributes).

### Commit 3 — literal decimal subscript fast path (59599749)

`a[1]=$i` spent 82ms of 613ms evaluating subscript "1" through the
full arithmetic pipeline. GNU arrayfunc.c:1368 runs evalexp on the
resolved subscript; for pure ASCII-decimal digits with a non-zero lead
every pre-evaluation pass is provably the identity, so the subscript
parses directly (rubash#156 whitelist-admission discipline; leading
zeros stay on the full path — 0NNN is octal).

### Numbers (release, A/B vs pristine 837514ff, median of 5)

| probe (20000 iters) | base ms | lane ms | delta | GNU ms | ratio |
|---|---:|---:|---:|---:|---:|
| p6-local3 | 2326 | 1552 | **-33%** | 115.5 | 20.1x -> **13.4x** |
| p6-null   | 519  | 339  | **-35%** | 35.1  | 14.8x -> **9.1x** |
| p6-arr    | 685  | 529  | **-23%** | 43.0  | 16.0x -> **12.0x** |
| nvm-load  | 612  | 595  | -3%      | —     | parse/dispatch-bound |

(local3/null/arr re-measured after commit 3: 1545 / 318 / 515ms —
commit 3 only moves arr.)

### Gates (all three commits)

- **matrix-attrs.sh** (20 sections: local -i math, frame roundtrips,
  nested frames, readonly blocks, declare -p rendering, local -p, attr
  flip/unflip, subshell+comsub export isolation, unset clears attrs,
  nameref/case/tempenv/declare -g/array-element/readonly-array):
  byte-identical base vs lane; vs GNU identical modulo $0 path and one
  ambient-env export line.
- **matrix-opts.sh** (15 sections: SHELLOPTS/BASHOPTS flips, readonly
  rejections, arith dynamic vars, attribute-aware arithmetic, assoc
  buckets, printf -v, subshell isolation, export -n): byte-identical
  base vs lane.
- **matrix-procenv.sh** (14 sections, new this round: exported vs
  unexported loop vars x arith-for/word-list, arith assignment writes,
  frame restore of exported/unexported/shadowed names, builtin
  diagnostic line numbers, SECONDS/PATH special sync, exported array
  element, tempenv, subshell): byte-identical base vs lane. Remaining
  GNU deltas are PRE-EXISTING and base-identical: (a) `export v; f() {
  local v=x; }` — GNU's local inherits att_exported so printenv sees
  the local inside the frame, rubash's mirror keeps the outer value;
  (b) an exported ARRAY leaks its storage serialization to printenv
  (GNU exports nothing for arrays).
- **Subscript edge matrix** (decimal/` 42 `/i128::MAX/`010`/`00` octal/
  signs/names/`1+1`/empty/`0x10` + runtime probes): identical to WSL
  GNU Bash 5.3.0.
- **true-baseline suites**: varenv, array, assoc, builtins, arith,
  param, new-exp, dynvar, printf, func, arith-for, traps, subst,
  errors, exec, exit, read — **18/18 at 0 diff lines** across the
  three commits' verification runs.
- cargo test --lib 545/545 (6 VarTable + 2 subscript unit tests new);
  --test regression 27/27; `RUSTFLAGS='-D warnings' cargo check
  --tests` and `--release --tests` clean; cargo fmt clean;
  src/lexer/continuation.rs untouched.

### Leftovers (measured, with owners)

1. **Per-command dispatch floor — the next deep subsystem.** null is
   now 9.1x GNU and its remaining 318ms is almost pure machinery:
   the execute_command chain (dispatch-kind matchers, trap consults,
   CommandNode handling, expand). nvm-load decomposition (exec-profile,
   405 commands): matcmd 48% of exec time, chain 24%, empty 12%. The
   lane brief's execute_cmd.c:624 function-pointer-table model
   (execute_command_internal dispatch as a table instead of the
   matcher chain) is the structural fix; not attempted this round.
2. **for-arith pair**: init/test/update still cost ~8us/iteration pair
   instrumented (163ms of arr's 808) — `arith_dynamic_values()` builds
   a 9-entry HashMap per evaluation (GNU expr.c resolves dynamic
   variables at find_variable call sites, no map build). Lazy
   resolution is the candidate.
3. **local-export inheritance** (matrix-procenv delta a above): GNU's
   make_local_variable (variables.c:2651) has the shadowed binding's
   att_exported survive into the local's child-env visibility; rubash
   keeps the outer value in the OS env. Semantic ticket, not perf.
4. **Exported array env leak** (delta b): exported arrays serialize
   their storage into the process env; GNU exports arrays only to
  
   bash-to-bash children via the array export mechanism (and modernish
   marks exported arrays as not crossing env at all). Ticket.
5. **PRE-EXISTING lexer bug found while gating — CAPTAIN
   (continuation.rs)**: `shopt -s extglob; shopt -u extglob` leaves
   the parser treating subsequent text as extglob — the next `echo
   $(( x + 5 ))` reports `syntax error near unexpected token '('`
   (reproducers: target/perf15/b*.sh prefix bisect; the pristine base
   binary reproduces byte-for-byte, so it predates this round). The
   gate matrices avoid extglob flips because of it.
6. **matrices note**: matrix-opts.sh section 3 shows GNU NOT running
   the `||` fallback after a failed readonly assignment
   (`SHELLOPTS=xxx 2>&1 || echo rc=$?`) while rubash runs it — the
   rubash#306 arith-error containment family (pre-existing,
   base-identical).

## perf17 round (2026-09-28, wt18/perf17 on aaa1d298): comsub park quadratic + dispatch-floor allocations

Two quantified deep-water items from the perf11/perf12 leftovers. All wall
numbers RELEASE build, alternating A/B against a pristine base binary
built this session from aaa1d298 in a sibling worktree (same load); GNU
anchors re-measured inside WSL this session (script files under
`target/perf17/`). Scratch instrumentation (env-gated comsub trace +
exec_profile P17 sub-phase timers) fully removed before commit.

### Task 1: the comsub park re-derivation (perf11 leftover #1, configure -n)

Decomposition (env-gated trace, `configure -n` full 24753 lines): 44198
`comsub_residuals_advance` calls walked 536 M chars; the 6018 park resumes
(park re-derivations) account for 534 M of them (99.6%), and ALL of that
is ONE park class — the backtick `<<` arm (tag=777 in the trace), not the
`$(` skip-fail parks perf11 suspected (those re-derived only 134 K chars
total). A single park created at the `<<_ACEOF` of `cat confdefs.h -
<<_ACEOF >conftest.$ac_ext` (configure:5488, inside the m4sh
false-positive backtick stretch) never cleared: 2406 re-derivations, each
re-walking the accumulated tail up to 525 K chars.

Root cause: that arm parked on `closes.is_none()` — but `closes` is the
heredoc-HEADER `)` closure signal (skip_heredoc_in_chars_with_closure's
second component), which is None for every heredoc whose header line
carries no `)` — i.e. every ordinary heredoc inside an open backtick —
even AFTER its terminator line arrived. The park was designed for the
terminator-not-yet-arrived case and never distinguished the two.

GNU anchor: make_cmd.c:512 `make_here_document` (driven by parse.y:3120
`gather_here_documents`) reads the body line by line through
`read_secondary_line` until a line equals the delimiter exactly, then the
reader continues PAST it — a consumed heredoc body is never re-read
(parse.y:3557 `read_token` streams). The aligned park condition is
therefore "terminator line not yet in the buffer", and that decision is
prefix-stable once found: the terminator search compares whole lines only
(the join loop appends complete '\n'-terminated physical lines), so a
longer buffer cannot move the first match.

Fix (class-level): `heredoc_scan.rs` gained
`skip_heredoc_in_chars_decided` (reports the `found_delimiter` local the
function already computed; the old 2-tuple signature stays as a wrapper so
the skip.rs / mod.rs / skip_parenthesized_unit_ex callers are untouched;
empty-delimiter early return keeps found=false, preserving its park).
`continuation.rs` (CAPTAIN-EXCLUSIVE — lane diff, commit bc4bc0e8,
awaiting captain review, process as perf8/perf9) parks on
`!terminator_found`. The `$(` skip-fail and esac-undecided parks and the
depth>0 `<<` arm are unchanged.

Numbers: configure -n 3413-3454 -> 1195-1214 ms (median 3416 -> 1202,
-65%; GNU 40-42 ms). Zero-semantic-change evidence: 546 lib (incl.
the comsub per-prefix incremental equivalence battery) + 27 regression +
`-D warnings` check + fmt; GNU-diff matrices
`target/issue-suites/results/perf17/matrix{1,2}.sh` (10 park shapes + 12
well-formed join shapes) byte-identical stdout vs GNU, stderr
byte-identical to the pristine base (the remaining heredoc-warning-text
diffs vs GNU predate this round); nvm.sh `-n` output and a posix-mode
per-iteration heredoc-in-backtick loop byte-identical base vs lane vs GNU.

### Task 2: the dispatch floor (perf12 leftover #1: "matcmd 48%, null 9.1x")

The lane brief asked whether execute_command_internal's dispatch is a
layered match/string-compare chain vs GNU's one switch at execute_cmd.c:624
on the COMMAND tag. Measured with scratch exec_profile sub-timers
(nested-exclusive where the command has no nesting — the 20000-`:` null
probe is exact): the string-match ladders are NOT the floor —
`execute_prepared_command`'s jobspec/restricted/is-disabled pre-match
2.1 ms, the primary builtin `match word` 0.9 ms, of a 483 ms total. The
floor is per-command BOOKKEEPING ALLOCATIONS in the matcmd tail and the
execute_command prelude:

- `update_underscore_parameter` -> `bind_underscore`: 67.7 ms of the
  71.9 ms matcmd tail (13.6% of the whole loop, 3.4 us/command). Every
  simple command cloned the whole EXPORTED_VARS marker string, split it,
  re-joined it, and re-inserted it (a byte-identical no-op once `_` is
  absent), plus a full Variable clone + a fresh `"_"` key to overwrite
  the variable's value. GNU's bind_lastarg (execute_cmd.c:4188) is ONE
  variable-cell bind plus a flag-bit attribute clear (VUNSETATTR
  att_exported, variables.h:124-133).
- `pre_alias_words = expanded.words.clone()`: a deep Vec<String> clone of
  every command's expanded words per command, taken to answer "did alias
  expansion change the words" — constant-false when the alias table is
  empty (GNU parse.y:3249 alias_expand_token consults the table; an empty
  table is one test).

Fix (both no-op-gated, the perf11 marker-list pattern):

1. `bind_underscore` (command_words.rs): the EXPORTED_VARS rewrite runs
   only when the borrowed list actually contains `_` or an empty fragment
   (the filter's two drop conditions — exactly the rewrite-when-changed
   gate, zero clones otherwise); the typed-store half swaps the value in
   place via get_mut behind VariableStore::set's only refusal condition
   (readonly), preserving the silent readonly-keeps-old-value behavior.
   The last-word COMPOUND_ASSIGNMENT_MARKER strip became a contains-gated
   Cow (GNU's lastarg is used as-is, execute_cmd.c:4746).
2. `execute_command` (command_execute.rs): `aliases.is_empty()` gates the
   raw-words Vec, the words clone, and the changed-words comparison —
   apply_alias_expansion_after_word_expansion is an identity move there
   (its own fast path), so the comparison result is unchanged.

Residual decomposition after the fixes (null probe, 20000 commands,
435 ms instrumented): bind_underscore 36.9 ms (1.8 us — the remaining
`"_"`/value key-value allocations and two map probes; structural to the
marker-string attribute model, perf12's structured-attributes leftover),
expand_command_words 31 ms, linecmd 11 ms, for-arith pair ~123 ms (the
largest single block, perf11's arith_dyn leftover), prelude remainder
~9 ms. nvm-load's matcmd is 41-48% of exec time but is NESTED execution
(the builtins/functions/comsubs inside matcmd), not the ladder: the fix
moves nvm-load ~-1% (527-534 -> 520-529 ms), as expected.

Numbers (alternating A/B, median): p6-null 380 -> 350 ms (-8%, GNU
36-45; ratio 9.5x -> 8.8x), p6-local3 1768 -> 1558 ms (-12%, GNU
118-119), nvm-load 533 -> 523 ms (flat; GNU 185-188), configure -n
3416 -> 1202 ms (Task 1's -65%, GNU 40-42).

Zero-semantic-change evidence (Task 2): 546 lib + 27 regression +
`RUSTFLAGS="-D warnings" cargo check --all-targets` + fmt; matrix4
(alias expansion + `$_` tracking + export list + unset, both shells):
byte-identical base-vs-lane AND lane-vs-GNU; `_`-tracking probes
(us-probe/us2/us3: readonly `_`, set -a exports, declare -x membership,
`$_` after commands) byte-identical base vs lane; OMB live load (real
HOME): PS1/declare -p/declare -F(284 functions)/alias md5 byte-identical
base vs lane. src/lexer/continuation.rs contains ONLY the Task 1 park
diff (bc4bc0e8, flagged for captain review).

### Leftovers (measured, with owners)

1. **CommandShape enum discriminant (execute_cmd.c:624 switch port)** —
   measured NOT worth it as a pure dispatch change: the Option-probe
   ladder + match-word self cost is ~3 ms of a 483 ms null loop. The fat
   CommandNode's probe ladders are cheap; the floor is the per-command
   allocations listed above plus the for-arith pair. A parse-time shape
   tag only pays if the tag also removes prelude predicate re-probing —
   defer until the allocation floor is exhausted.
2. **bind_underscore residual 1.8 us/command**: two key/value String
   allocations + env_vars HashMap probe + BTreeMap get/get_mut per
   command. Removing it needs `_` as a first-class maintained slot (GNU
   keeps one SHELL_VAR cell) — structured-attributes family, perf12.
3. **errexit/xtrace maintained flags**: errexit_enabled() costs two env
   gets per call and runs 2-3x per command; GNU tests an int
   (`exit_immediately_requested`). The `__RUBASH_ERREXIT`/`XTRACE` flip
   sites are 8+ across shell_options/embedded_mutations/
   command_substitution/compound_exec (including wholesale subshell
   save/restore lists) — a maintained Cell must cover all of them; do it
   as its own reviewed change, not a lane rider.
4. **configure -n now ~29x GNU (1202 vs 41 ms)**: remaining gather cost is
   the other scanners' parks on the same 525 K-char m4sh backtick stretch
   (quotes/balanced/fnbody advance over the tail per candidate line) and
   the token-level keyword stack — the C/E quote-leak family
   (continuation.rs, captain) is the semantic owner of why the stretch
   reads as one open construct at all.

## qleak round (2026-09-30, wt19/qleak on e3623c68): full-suite 2x inventory + pointer execution of compound bodies

Owner goal "all suites 2x". First full-suite inventory on a RELEASE binary
(all previous rounds' suite tables were debug), then one structural fix.

### Inventory (release, scripts/run-perf-suite.sh, median; GNU = WSL 5.3.0
inner-timed, same session; host had parallel lanes — GNU medians consistent
with prior rounds)

| probe | rubash ms | GNU ms | ratio | bucket | main cost attribution (prior rounds' decompositions) |
|---|---:|---:|---:|---|---|
| 01-startup-empty | 73 | 5 | 14.6x | >10x | ~54ms MSYS-parent spawn constant (perf10: cmd.exe pays it identically); rubash in-process floor ~14ms |
| 02-startup-fndef | 67 | 5 | 13.4x | >10x | same parent-spawn constant |
| 04-loop-true-builtin | 167 | 13 | 12.8x | >10x | distributed per-command floor (perf17: bind_underscore 1.8us, expand 31ms/2k cmds, for-arith pair) |
| 05-arith-x5000 | 138 | 12 | 11.5x | >10x | arith_dyn snapshot remains 87ms-class (perf11/exphot) + loop machinery |
| 06-strconcat-x5000 | 301 | 23 | 13.1x | >10x | assignment apply + walker tail (exphot) |
| 07-fncall-noop | 354 | 31 | 11.4x | >10x | frame machinery; local3-family leftovers (perf12) |
| 08-cmdsub-true | 135 | 331 | 0.4x | <2x | spawn parity |
| 09-external-uname | 92 | 205 | 0.4x | <2x | spawn parity |
| 10-pathmiss-x100 | 141 | 20 | 7.0x | 2-10x | PATH lookup + `command -v` miss path |
| 11-pipeline-yes-head | 191 | 7 | 27.3x | >10x | decomposed this round: ~73ms harness parent-spawn constant (floor = probe 01); head-file\|head costs the SAME ~160ms in Git Bash (msys binary cost, not rubash); python-parent yes\|head completes in ~35ms — rubash's residual ~120ms is its own pipeline spawn/wire/wait machinery vs GNU's fork. 2x is harness-bound regardless |
| 12-pipe-echo-read | 1535 | 902 | 1.7x | <2x | — |
| 13-readloop-gen | 206 | 23 | 9.0x | 2-10x | read builtin + simple-command dispatch (perffix family) |
| 14-glob-srcrels | 2652 | 38180 | 0.1x | <2x | env-bound (drvfs globbing), regression tracking only |
| 15-expansion-x5000 | 562 | 58 | 9.7x | 2-10x | empty_rhs walker tail + apply (exphot decomposition) |
| 16-parse-flat8000 | 449 | 18 | 24.9x | >10x | 56us/cmd: chain 235ms of 435ms wall (this round's profile), assignment apply 32ms, jobs 14ms, linecmd 10ms — distributed floor |
| 17-parse-flat8000-n | 127 | 12 | 10.6x | >10x | batch tokenizer + parse loop (perf15) |
| 18-nested-brace-nst1 | 206 | 5 | 41.2x | >10x | THIS ROUND: O(depth^2) body clones in brace-group executor |
| 19-nested-brace-nst2 | 276 | 6 | 46.0x | >10x | same + two-line parse cost |
| 20-as-fn-mkdir-p | 123 | 7 | 17.6x | >10x | function def + call machinery |
| 21-configure-head-n | 101 | 8 | 12.6x | >10x | gather + tokenize + nested body re-parses (perf15) |
| 22-configure-full-n | 1206 | 40 | 30.1x | >10x | gather-dominated: scanner parks on the m4sh backtick stretch — continuation.rs (captain, C/E quote-leak family) |
| 23-nvm-parse-n | 234 | 18 | 13.0x | >10x | parse-bound: tokenize + 1500 nested body re-parses (perf7/15) |
| 24-nvm-load | 539 | 34 | 15.9x | >10x | gather+parse (as 23) + per-command exec; compound-body clones (THIS ROUND) |
| 25-yes-head-read | 1270 | 675 | 1.9x | <2x | — |

Buckets: <2x = 5 probes (08 09 12 14 25); 2-10x = 3 (10 13 15); >10x = 16.
The 01/02/11 ratios are ~80% MSYS-parent spawn constant (GNU pays nothing
comparable inside WSL) — like probe 14 they are environment-bound for the
2x goal; the honest in-process gap for 01 is ~14ms vs 5ms.

### Landed change: compound bodies execute by POINTER, not by clone

Root cause (found this round, the largest non-captain item): EVERY compound
executor deep-cloned its body into `Ast { commands: body.clone() }` before
executing. GNU walks compound bodies by pointer — execute_cmd.c:624
`execute_command_internal` dispatches on the COMMAND tag of the node
make_cmd.c allocated ONCE at parse; execute_while_or_until (execute_cmd.c:3796,
`execute_command (while_command->test)` at :3809), execute_for_command
(:2990), execute_case_command (:3644), execute_if_command (:3862),
execute_function (:5181) all re-walk the same COMMAND pointers and GNU never
copies a COMMAND at execution time. In rubash the clone cost was:

- **brace groups: O(depth^2).** `execute_brace_group_pipeline` cloned BOTH
  the whole CommandNode (`command.clone()`, whose brace_group body holds the
  remaining depth-k nodes) AND `brace_group.body.clone()` per level —
  200-deep `{ { ...; } }` cloned ~2*sum(depth) CommandNodes of ~2.2 KB each
  per run (measured shape: exec 623ms vs -n parse 103ms at D=400; the
  exec-profile counters saw only 1.4ms — the time was pure clone churn).
- **for loops: per ITERATION.** loop_select.rs cloned the body Vec every
  iteration (GNU execute_for_command walks the same pointer per iteration).
- while/if/case conditions and bodies, subshell bodies: one deep clone per
  execution, all pure waste when no redirect splice is needed.

Fix (root cause: the Vec-wrapper ownership, not a guard): `execute_ast_inner`
now takes `&[CommandNode]` (execute_ast keeps `&Ast`; the ~29-function
matcher chain that shares the command list — alias_introduced_*, simple_if,
skip_and_or_rhs, pipeline walkers — was mechanically converted to slice
params, compiler-guided). Every compound executor now calls the slice entry
on the stored body. The two clones that feed real mutations are gated on the
mutation's own necessary condition (rubash#117 whitelist discipline):

1. `command.clone()` for `materialize_compound_output_process_substitutions`
   runs only when a redirect target starts with `>(` (both of its arms —
   shared_combined_output_process_substitution and
   materialize_compound_output_redirect — require that prefix; without it
   the call is the identity returning an empty vec).
2. `body.clone()` for `apply_brace_group_redirects` /
   `apply_command_output_redirects` runs only when a stdio-slot redirect
   exists (their four `if let` splice admissions: redirect_out/append with
   fd==1, redirect_err/redirect_err_append with fd==2; subshell uses the
   conservative any-slot superset since numbered entries also splice).
3. The `normalize_inline_compound_commands` rewrite for while/if bodies runs
   only when some command's first word is `for` (its only rewrite arm,
   inline_arithmetic_for_command, requires exactly that).

A mid-refactor regression was caught by the probe battery: the mechanical
`ast.commands` -> `commands` rewrite made execute_simple_pipeline's stage
collector shadow the parameter, returning None for every pipeline ("echo hi |
cat" failed) — the shadow audit (functions with a `commands` param AND a
local `commands`) found this one site; fixed by renaming the collector.
Keep that audit in mind for any future sed-style refactor of this chain.

### Numbers (release, alternating A/B vs pristine e3623c68, best-of-3;
harness medians from the full-suite re-run in parens)

| probe | base | lane | delta | ratio |
|---|---:|---:|---:|---:|
| 18-nested-brace-nst1-d200 | 188-211 | 63-78 | **-66%** | 41.2x -> 16.8x (harness 206->84ms) |
| 18 shape D=400 | 615-629 | 94-98 | **-85%** | now LINEAR in depth |
| 19-nested-brace-nst2-d200 | 241 | 116 | **-52%** | 46.0x -> 23.7x (harness 276->142ms) |
| 24-nvm-load | 517 | 446 | **-14%** | 15.9x -> 13.6x (harness 539->461ms) |
| 12-pipe-echo-read | 1478 | 1491 | flat (parity) | 1.7x |
| 04/05/06/07/15/16/17/20/21/23 | — | flat | noise band | — |

Probe 19's remainder is parse-side: `-n` 120ms ≈ exec 119ms at D=200 (the
exec-side clones are gone; the ~60ms-above-floor is the nested body
re-parse fan-out — perf15 leftover #2, the whole-parser architecture item).
Probe 16's chain phase (235ms of 435ms) decomposes into the same distributed
per-command floor perf17 profiled; no single >5% site remains.

### Semantics gate (zero-change evidence)

- cargo test --lib 546/546; --test regression 27/27; `RUSTFLAGS='-D
  warnings' cargo check --tests` and `--release --tests` clean; cargo fmt.
- src/lexer/continuation.rs untouched (verified: git diff empty for it).
- GNU-diff matrices m1-m3 (`target/issue-suites/results/qleak/`, script
  files run on base binary, lane binary, and WSL GNU 5.3.0): brace groups
  with stdout/append/stderr redirects and nesting, subshells (redirects,
  var isolation, cwd restore, capture), if/elif/else, while/until
  (break/continue in condition and body, `while case ... break`), word-list
  and arithmetic for, the collapsed inline-`for ((...))` form inside while
  bodies, case (incl. `;;&` fallthrough), pipelines (builtin-builtin,
  builtin-compound, external-external, read loops), loop-body redirections
  — base-vs-lane byte-identical stdout AND stderr AND rc on all three.
  lane-vs-GNU: m2/m3 fully identical; m1 has ONE pre-existing divergence
  (`{ echo x >&2; } 2>&1 1>/dev/null` loses the message; the pristine base
  binary reproduces it byte-for-byte).
- true-baseline suite slices (WSL oracle, both binaries): braces, case,
  func, errors, read, quote — base-vs-lane stdout+stderr byte-identical,
  GNU diff counts unchanged (6/4/0/4/30/94).
- nvm `--no-use` load gate: declare -p NVM_*, declare -F nvm_* count,
  compgen count, type nvm, nvm --version, alias count — byte-identical
  base vs lane (only the embedded $$ differs, per-process).

### 2x roadmap (per bucket, what each row still needs)

- **Under 2x already (5)**: 08 09 12 14 25 — hold.
- **2-10x (3)**:
  - 10-pathmiss (7x): PATH-miss scan cost; candidate = lookup_paths.rs
    probe result caching (GNU's command -v miss is a cached-hash miss).
  - 13-readloop (9x): read-builtin + dispatch floor, same family as 04.
  - 15-expansion (9.7x): exphot leftovers — the `"$i"`-word GNU
    param_expand port and the RHS double-pre-scan memo; each measured
    single-digit %; reaching 2x needs the borrow-based expansion refactor.
- **Over 10x, environment-bound (3)**: 01 02 11 — the MSYS-parent spawn
  constant (~54ms) floors the harness ratio near ~11x even at zero rubash
  cost; GNU's WSL inner timing pays nothing comparable. Not fixable in
  rubash; needs a native-parent harness or acceptance as environment noise.
- **Over 10x, parse-bound (5)**: 17 21 23 (10-13x) and 19's remainder —
  the nested body re-parse fan-out (GNU parses bodies inline in the same
  reader pass; the fix is a single-pass recursive-descent body parse, a
  whole-parser architecture change) plus the join-loop scans perf7
  characterized. 22 (30x) additionally needs the captain-family scanner
  false positives on the m4sh backtick stretch (continuation.rs C/E
  quote-leak family).
- **Over 10x, per-command floor (6)**: 04 05 06 07 16 20 24's exec half —
  the distributed machinery perf11/12/17 profiled: the for-arith pair
  (arith_dyn maintained-entry model), bind_underscore residual 1.8us,
  marker-string attribute leftovers, expansion walker tail. No single
  greater-than-5% site remains; reaching 2x needs the already-ticketed
  deep borrow/attribute subsystems.

## perf19 round (2026-09-30, wt19/perf19 on e3623c68): heredoc body opacity + case-pattern closers + history-driver token reuse

The configure -n 29x follow-up (perf17 leftover #4). The brief's
hypothesis ("the other scanners' parks on the same backtick stretch")
was HALF right — the parks were real but the semantic owner was not
the backtick family at all. All wall numbers RELEASE build,
alternating A/B against a pristine e3623c68 base built this session
in a sibling worktree (same load); GNU anchors re-measured INSIDE
WSL this session (script files under `target/issue-suites/results/perf19/`).
Scratch instrumentation (env-gated scanner/park/phase trace, RUBASH_PERF19)
fully removed before commit.

### Decomposition of the 1247 ms (env-gated trace)

| Phase | Time | Detail |
| --- | --- | --- |
| comsub residual advance | 671 ms | 197 M chars / 3516 calls |
| group parse (run_history_group) | 333 ms | includes re-lex per group |
| feeder push_line | 95 ms | 24753 lines |
| other 6 text scanners | ~13 ms | NOT the bottleneck |
| keyword stack (token_more) | 0 ms | already incremental |

And the structural fact behind it: the group at configure:5176
absorbed the ENTIRE remaining file — 19578 lines, 525 K chars — as
one never-completing group (1082 groups before it, then one giant).

### Root cause 1: heredoc BODIES were live text to the group scanners

The group driver freezes its parked scanners while the heredoc-body
queue is non-empty, then the first candidate line AFTER the
terminator re-walks the checkpoint across header+body+terminator.
At top level (no open `$(`/backtick) the scanners had NO heredoc
skip at all: the body's characters toggled live quote state. The
apostrophe in the `<<_ACEOF` C commentary `can't` (configure:5207
body) held `single` open; with state corrupted, the m4sh comment
`add `-static'` (configure:5405) never entered comment state and its
backtick parked `backtick` for the rest of the file. GNU never lexes
a body: `make_cmd.c:512 make_here_document` (driven by parse.y:3120
`gather_here_documents`) reads bodies through `read_secondary_line`
with no quoting state, and `parse.y:3557 read_token` streams past
the terminator (bodies are also what make GNU read the file as 4000
independent groups).

Fix (all six scanners in lockstep with their oracles):
`heredoc_scan.rs` `skip_heredoc_top_level` (comsub_context=false core:
the two PST_EOFTOKEN pushbacks disabled — `EOF)x` is body text at top
level per make_cmd.c:571-574/602-611; refuses empty delimiters and
headers carrying an open `$(`/`${/backtick introducer — upstream
heredoc7.sub's `cat <<EOF && grep $(` shape) + a top-level `<<` arm
in comsub/quotes/balanced/subscript/close_char (parked + oracle),
parking only while the terminator line has not arrived (perf17's
prefix-stable rule). `<<<` is one operator (parse.y:3690-3706) and is
consumed atomically — the randomized incremental batteries caught the
second-`<`-re-read regression before it shipped.

### Root cause 2 (exposed by fix 1): case-pattern `)` popped subshell closers

With bodies opaque, configure died at configure:23345 from `(` at
configure:23329 — ALSO on the pristine base in isolation (pre-
existing, previously masked by the mega-group). GNU parse.y:1037
case_command consumes a pattern's `)` as a case token; the close-char
scanner now runs the STAGED case machine (skip.rs's
update_command_substitution_case_depth, parse.y:3369-3386 +
parse.y:3433-3441 empty case) with a per-delimiter depth snapshot
(`case_depth_at_push`): `)` closes only at its push-time depth. The
staged variant is required — the unstaged one loses the reserved-word
boundary after the case subject and never counts `case W in esac`'s
ESAC (`( case x in esac )` held the subshell open). Keyword/operator
tracking is gated by `close_char_operator_context`: live only at top
level / `$(` / backquote / funsub / command-position subshell — not
inside quotes, `${param}`, array lists, or arithmetic (`$((1<<2))`,
parse.y:3727).

### Fix 3: the history driver threw away perf10's certified tokens

`run_history_group` re-lexed every group although `read_next_source_group`
had just committed a certified-equivalent stream. When nothing rewrote
the text (history expansion inert AND alias expansion identity) the
stream is reused (`run_source_pre_lexed_with_line_offset`), the
per-group unclosed pre-scan is skipped for pre-lexed (complete)
groups, and the per-line history-record clone is gated on history
being on. GNU anchor parse.y:3557 read_token streams once.

### Numbers (median of alternating A/B)

| Probe | base (e3623c68) | lane | GNU (in-WSL) | ratio |
| --- | --- | --- | --- | --- |
| 22-configure-full-n | 1188 ms | **420 ms (-65%)** | 38-39 ms | ~31x -> **~11x** |
| 21-configure-head1374-n | 83 ms | 76 ms | (9 ms class) | flat |
| 23-nvm-parse (-n) | 228 ms | 229 ms | 16-17 ms | flat |
| 24-nvm-load | 465 ms | 462 ms | (185-188 class) | flat |

configure -n rc=0, stdout/stderr byte-empty on both sides.

### Zero-semantic-change evidence

- 546 lib (the incremental-equivalence batteries drove out two
  in-development regressions: `<<<` re-read, unstaged-case boundary)
  + 27 regression + `RUSTFLAGS="-D warnings" cargo check --all-targets`
  + fmt.
- GNU-diff matrices (stdout+stderr+rc, WSL GNU 5.3.0):
  `target/issue-suites/results/perf19/matrix{1,2,3}.sh` — 14
  heredoc-body shapes, 14 case-closer shapes, the history-driver
  grouping shapes: ALL byte-identical to GNU; matrix3 fails on the
  pristine base (the case-blind break), matrix1/2 pass on both.
- Suite slices via true-baseline.sh (heredoc/case/comsub/comsub2/
  quote/redir): lane diff-vs-GNU line counts IDENTICAL to base on
  every suite (all remaining diffs pre-existing).

### Leftovers (measured, with owners)

1. **configure -n residual ~420 ms**: parser 141 ms + feeder 104 ms
   + scanners 26 ms + noexec walk 14 ms + driver ~40 ms. The parser
   and feeder are the parsearch #281/#292 nested-body re-parse
   family (next-round architecture; GNU does both in ~39 ms total).
2. **Pre-existing (reproducer `printf '( case x in esac )\necho ok\n'`):
   the EMPTY case inside a subshell fails identically on base and
   lane (GNU rc=0) — tokenizer/parser family, distinct subsystem;
   both this round's matrices and the suite slices confirm it is not
   the group scanners.
3. **fnbody scanners still scan heredoc bodies** (same class as the
   six fixed here, but fnbody's is_open needs a function-signature
   prefix, so no configure-family corpus is known to block on it) —
   cover it when a reproducer exists.

## parse20 round (2026-09-28, wt20/parse20 on 0fe8d0fc): nested-body re-parse decomposition + if-family inline sections

The "解析绑定桶" lane. The brief hypothesized configure -n's residual
~120 ms (parser 141 + feeder 104) and nvm's parse side were dominated by
nested-body re-parsing (parsearch: 1577 re-parses, ~150 ms; GNU inlines
compound bodies in the same recursive-descent pass). All numbers RELEASE
build, alternating A/B against a pristine 0fe8d0fc base binary built
this session from master sources (private target dir, same load). Scratch
instrumentation (env-gated phase/scan timers + atomic walk counters)
fully removed before handoff.

### Decomposition (instrumented lane binary; the brief's ask #1)

nvm -n (wall ~229 ms): parse:top 64.2 ms (ONE top parse — the whole file
is a single history group; 2130 nested parse entries, max depth 11);
lex:all-entries 89.8 ms/491 calls/0.20 MB. Body re-parse shapes
(inclusive): if 1218 calls/46.7 ms, case 256/22.6, fn-strict 125/56.4,
loop 42/3.4, for 13/2.7, subshell 15/1.3; folded re-lex shapes are tiny
(fnbody 30/3.1 ms, bracegroup 31/0.8). Boundary scans: if_section 658
calls/32.9 ms inclusive, matching_brace 96/2.2, find_if_then 560/0.73,
case_body_end 256/0.71. Folds (gated) 2.0 ms, extglob marking 0.6 ms.
Walk counts: main-loop iterations 14,375 vs boundary-scan iterations
47,982 (3.3x).

configure -n (wall ~420 ms): parse:top 2481 calls/144.5 ms; nested
entries 6793. Shapes: if 4387/255-260 ms, case 1336/77.5, for 165/35.1,
loop 20/9.6, fn-strict 24/5.6; relex:bracegroup-folded 623/14.5.
if_section scans 2636/241 ms inclusive. Walk counts: main loop 40,405 vs
scans 141,631 (3.5x).

**Model revision (the round's key finding).** The walk amplification is
real (3.3-3.5x) but the scan walkers are CHEAP: after parsearch's
Rc-threading/fold-exit/single-scan work, the double walk (scan for the
closer + re-parse the slice) costs only ~10-25 ms on configure and ~5-10
ms on nvm. The actual owners of the remaining wall:

| Phase (configure -n) | ms | owner |
| --- | ---: | --- |
| driver gather (read_next_source_group family) | 178 | feeder join/re-lex loop — #281/#292 captain family (continuation.rs adjacent) |
| feeder push_line (26331 lines) | 127 | ditto — scanners inside it only 24.7 (comsub 7.7 + compound 7.0 + quotes 5.7 + param 4.3) |
| parse: whole | 144 | this lane's bucket, see below |
| parse: handle_token | 84 | word intake (push_command_word 50, of which WordScans::run 31) + per-kind machinery 34 |
| parse: try_parse_compound_start | ~52 exclusive | miss path only 8.9 (36878 misses); rest is compound-hit pre/post work |
| noexec walk + driver + startup | ~95 | executor/driver |

GNU does configure -n in ~38 ms total; its parser runs ZERO per-word
metadata scans at parse time (words are stored once, scanned at
expansion) and reads every token exactly once.

### Landed change: if-family inline sections (the GNU single-pass model)

`parse_loop.rs` + `if_command.rs` (+2 satellite caller lines in
function_command.rs/coproc_command.rs; +388/−63 total). The main command
loop is extracted into `run_command_loop(tokens, state, start)` (shared
by whole parses and sections); `run_inline_section` swaps in a fresh
command list plus the per-body balancer state the old fresh ParseState
started with (pending_comsub 0, in_subshell false, stray-close leniency),
runs the shared loop until the closer predicate fires, restores the
caller's state, and applies the same fold/extglob tail the slice parse
ran. `SectionStop::{IfCondition,IfBody}` reproduce find_if_then /
parse_if_section's exact stop predicates (stack-is-empty +
command_boundary_keyword_allowed + stop set); the loop's incremental
compound-boundary stack is truncated at compound jumps (a successfully
parsed compound's frames are net balanced, so the stack matches the
scanner's at every stop candidate). Errored bodies fall back to the cold
scan (scan_if_section_end / find_if_then kept) so the boundary the old
slice parse used is preserved. GNU anchor: parse.y:1037-1054 if_command
— the condition, `then`, the bodies and `fi` are shifted by the SAME
yyparse run; parse.y:1264 compound_list. Satellites (function/case
compound bodies, time-prefixed compounds) run the same engine over a
scratch state via parse_if_command_standalone (contract-equal to the old
parse_body_with_diagnostics fresh state).

### Numbers (median of alternating A/B, steady state)

| Probe | base (0fe8d0fc) | lane | note |
| --- | ---: | ---: | --- |
| configure -n | 415 ms | 409 ms | −1.5% |
| 23-nvm-parse-n | 218 ms | 219 ms | flat |

The walk amplification for if-sections is eliminated (the scan+slice
double walk is gone), but the scans were cheap walkers, so the wall
moves little. **The lane target (configure <250 ms, nvm <8x) is NOT
reachable from the parse side alone**: the feeder join/re-lex family
(~300 ms of configure's gather+push_line) is the #281/#292 captain
family, and the parse-side remainder is per-word scan machinery.

### Zero-semantic-change evidence

- 546 lib + 27 regression + `RUSTFLAGS="-D warnings" cargo check
  --all-targets` + cargo fmt --check, on the final clean tree.
- 32-case 3-way matrix (GNU vs base vs lane, stdout+stderr+rc):
  base-vs-lane byte-identical on ALL 32 (artifacts:
  `target/issue-suites/results/parse20/cases/`); gnu-vs-lane 22/32
  byte-identical, 10 diffs are pre-existing master diffs (identical on
  base; error-wording/path prefix and `time` output class).
- true-baseline.sh slices (errors/cond/func/case/comsub/quote/heredoc/
  dstack): lane diff-vs-GNU line counts IDENTICAL to base on every
  suite.
- configure -n / nvm -n: rc=0, stdout+stderr byte-empty, base vs lane
  outputs identical.
- src/lexer/continuation.rs untouched (git diff 0 lines).

### Leftovers (measured, with owners)

1. **Feeder join/re-lex ~300 ms of configure -n** (gather 178 + push_line
   non-scanner ~102): the #281/#292 resume/refuse family, parked
   scanners' bookkeeping and the join loop's per-line re-tokenize —
   captain family (continuation.rs exclusive). GNU reads once
   (parse.y:3557 read_token).
2. **Per-word parse-time scans 31 ms configure / 19 ms nvm**
   (WordScans::run inside push_command_word): GNU runs NO parse-time
   word scans; the parsearch-deferred triple-store/field-type churn
   (word string stored 3x: token.value + cmd.words[i] +
   metadata.value/.raw) is the deep fix — needs WordMetadata field-type
   changes across 84 sites, next-round scale.
3. **Compound-hit overhead ~43 ms configure** (try_parse_compound_start
   exclusive beyond the miss path): pre/post body work per compound
   (metadata assembly, finish_compound_command redirect walks).
4. **Inline extension to case/for/loop/fn bodies**: measured scanner
   cost is 3.2 ms (case_body_end) / <1 ms (for/loop) on configure —
   sub-noise ROI; do it for architecture consistency only when the
   satellite states thread naturally (parse_function_compound_body,
   parse_time_prefixed_compound_command still use scratch states).
5. **Errored-section extglob marking spans [start..error_i]** instead of
   the old [start..scan_boundary] — diverges only when a body errors
   BEFORE a `shopt -s extglob` that sits between the error and the
   closer (no known corpus; noted for completeness).
## envfix3 round (2026-09-30, wt20/envfix3 on 0fe8d0fc): 2-10x bucket (10/13/15) + the MSYS-parent dual caliber

Owner goal "all suites 2x". Three items from the qleak inventory's 2-10x
bucket plus the environment-bound bucket's native-parent calibration. All
numbers RELEASE, alternating A/B vs a pristine 0fe8d0fc base built this
session in a sibling worktree (../rubash-wt-envfix3-base); official
medians via scripts/run-perf-suite.sh (--runs 7); native-parent medians
via the new scripts/run-perf-native-parent.py (--runs 10). GNU anchors
re-measured inside WSL this session by the suite (10-pathmiss 21ms,
13-readloop 23ms, 15-expansion 57ms, 04 13ms). Scratch instrumentation
(phase timers, site-key traces) fully removed before commit.

### #10 pathmiss (was 7.0x): the miss walk re-derived its candidates per DIRECTORY

Decomposition (instrumented base): 100 unique-name misses cost 43.2ms in
find_user_command alone = one 10.5ms first-fill (69 read_dirs for the
host PATH) + ~310us per warm miss. The per-miss cost was pure churn in
find_in_path_via_listings: for EVERY directory it rebuilt dir.join(name),
then with_extension(ext) + file_name().to_string_lossy().to_lowercase()
per PATHEXT candidate (~8 strings x 69 dirs re-derived from the same
name). The listing cache (rubash#159) was already warm; only the
candidate DERIVATION was inside the loop.

Fix (path.rs): the candidate set is a pure function of NAME + PATHEXT
(GNU findcmd.c:623 find_user_command_in_path passes the same NAME to
find_in_path_element for every directory), so listed_candidates derives
the lowercased listing keys and spelled file names ONCE per lookup; the
walk is now one allocation-free HashSet::contains(&str) per directory x
candidate, with the dir.join + is_file confirmation only on a listing
hit. Candidate order and spellings byte-identical to the old
executable_candidate_listed (matrix4).

Numbers: harness median 132 -> **94ms** (-29%, GNU 21: 6.3x -> **4.5x**);
native-parent median 64.3 -> **34.2ms** (GNU 21: 3.1x -> **1.6x**).
Steady-state warm miss ~310us -> ~20us class.

### #15 exphot leftovers: `:=` single-evaluation memo (a correctness fix) + the `"$name"` word fast path

**`:=` memo (exphot leftover #2, landed as a GNU-parity bug fix).** GNU
parameter_brace_expand (subst.c:9777) evaluates each `${}` occurrence
exactly once: the `:=` arm (subst.c:10346 `case '='` with check_nullness
-> parameter_brace_expand_rhs) expands the alternate and assigns in one
call. Rubash's layered passes (assignment-RHS pre-scan
assignment_expansion.rs:840, walker pre-scan embedded_mutations.rs:343,
and the real operator arms) each re-derive the same fragment, and when
the alternate resolves to NULL the `value non-empty` set-check fails in
every pass, so the alternate RE-EXECUTES: `v=${v:=$(echo HI >&2)}`
printed HI **4x** (GNU 1x; matrices under
target/issue-suites/results/envfix3/). Fix: the pre-scan stores the
applied fragment's resolved value under the same (word-ctx, frag-path,
text) site key the SubXpassFrame memo uses (new ASSIGN_APPLIED map in
expand_braced_indices.rs, cleared when a fresh outermost WordCtxGuard
installs, so sibling words and loop iterations never share entries);
later passes - the walker pre-scan and both real-arm `:=` resolutions
(parameter_words.rs ops.split_colon_pair(name, b'=') arm and
expand_braced_ops.rs) - reuse it. stderr is now byte-identical to GNU on
every :=/=/comsub side-effect case; matrix1/matrix2 stdout byte-identical
base-vs-lane. (The RHS double-pre-scan memo itself stays as exphot
specified - the memo keys the application, which subsumes it for the
divergence that mattered.)

**`"$name"` whole-word fast path (exphot leftover #1).** Measured base
release cost of one quoted-parameter word (`: "$i"` vs `: i` loops,
5000x): ~9-10us/word (exphot's debug 24us/word shrinks but survives).
GNU param_expand (subst.c:10464) switches on the character after `$`;
for a name it resolves the variable in one arm. The port (command_
prepare.rs expand_quoted_single_name_parameter_word): whitelist
admission - cooked word EXACTLY `$<shell-name>` + PARAM_NAME_END_MARKER
AND raw EXACTLY `"$<name>"` (embedded text, escapes, `${...}`,
positional digits, the scalar specials and the word-list `$@`/`$*` all
fall through) - then the walker's own `$name` arm (embedded_mutations.rs
`Some(first) if is_shell_name_start`): the same
dynamic_parameter_value -> shell_variable_value -> exact_case_env_var
chain, shell_safe_value, the same walker-tail marker restores and the
same strip_ifs_protection_markers. Skipped guard steps are provably
identity for this shape (no `${`, no `$(`, no backtick, no braces, no IFS
marking, nounset pre-scan runs per-command upstream, split/null tail
gates key on UNquoted words). Word cost ~9us -> **~1us**; `"$i"` in a
test command is now free vs a literal word.

Probe 15 median (harness): base 572 -> lane 593 - flat within the host's
+-10% noise band (the test-word win is ~20ms of 570; the strip fragments
and loop floor dominate). Probe 04: 162 -> 172, same band.

### #13 readloop (was 9.0x): decomposed - the wall is the PRODUCER, not read

Direct-spawn decomposition (base binary): the reader half
(`while read -r l; do :; done < file`, 2000 lines) costs 42ms work
(~21us/iter read+colon - the read builtin's own IO path is healthy, ~1.3x
GNU's per-read), and it runs CONCURRENTLY with the producer. The
producer (`while [ ... ]; do echo "..."; i=$((i+1)); done`) is the
critical path: loop machinery (test+arith) ~96ms/2000 iters (~48us/iter,
the 04/05 per-command floor family: perf11/12/17 profiled it to "no
single >5% site; needs the ticketed borrow/attribute subsystems") +
echo's buffered-output path ~16us/call. Read-side improvements cannot
move the wall (reader has 4x headroom); lane median 204 vs base 203 -
flat, as predicted. The honest owner of 13's remaining 8.9x is the
04/05/06 floor family, not `read`.

### Environment bucket (01/02/11): native-parent dual caliber

New instrument: `scripts/run-perf-native-parent.py` - a native Python
parent spawns rubash.exe via CreateProcess, timing only the child (the
MSYS fork/exec constant never enters the window). GNU side stays the
suite's inner-WSL numbers. Dual-caliber medians (base/lane identical for
01/02/11; lane shown):

| probe | MSYS parent (suite) | native parent | GNU inner | MSYS ratio | native ratio |
|---|---:|---:|---:|---:|---:|
| 01-startup-empty | 73 | 16.2 | 5 | 14.6x | **3.2x** |
| 02-startup-fndef | 67 | 11.9 | 5 | 13.4x | **2.4x** |
| 10-pathmiss (lane) | 94 | 34.2 | 21 | 4.5x | **1.6x** |
| 11-pipeline-yes-head | 191 | 138.4 | 7 | 27.3x | **19.8x** |
| 13-readloop (lane) | 204 | 150.9 | 23 | 8.9x | 6.6x |
| 15-expansion (lane) | 593 | 531.0 | 57 | 10.4x | 9.3x |

Verdict vs the plan's "<2x then re-host the suite" condition: 01/02 do
NOT reach 2x natively (16.2/11.9 vs 5), so the suite's rubash side stays
MSYS-parented; the dual caliber is recorded here instead. The ~54ms MSYS
constant IS confirmed as the bulk of the ledger's 11-27x, but the
corrected residuals are rubash-owned: 01/02's ~11-16ms in-process startup
init vs GNU's 5ms, and 11's ~130ms internal yes|head pipeline machinery
(vs GNU's ~7ms fork+wiring - matches the qleak decomposition's "rubash's
residual ~120ms is its own pipeline spawn/wire/wait machinery"). Only the
parent-spawn share of the old bucket is environment noise; the corrected
classification: 01/02 = startup-init work (2.4-3.2x), 11 = pipeline
machinery (>10x), neither is harness-bound at its core.

### Semantics gate (zero-change evidence)

- 546/546 lib, 27/27 regression, `RUSTFLAGS='-D warnings' cargo check
  --tests` and `--release --tests` clean, cargo fmt.
- src/lexer/continuation.rs untouched (git diff empty for it).
- GNU-diff matrices `target/issue-suites/results/envfix3/matrix{1..4}.sh`
  on pristine base, lane, and WSL GNU 5.3.0: matrix1/3/4 base-vs-lane
  byte-identical stdout+stderr (matrix3's only GNU deltas: `$$` pid and
  /d vs /mnt/d path forms - environment, pre-existing); matrix2
  (side-effect visibility) stdout identical and lane stderr now
  byte-identical to GNU while the base ran `:=` comsubs 2-5x (the fixed
  divergence); matrix4 (PATH-scan: first-dir-wins, extension order,
  dotted/case-insensitive names, miss class, hash views, fingerprint
  invalidation, nonexistent PATH entries) byte-identical base-vs-lane.
- true-baseline slices (exp new-exp more-exp posixexp quote read): lane
  rb.out/rb.err/rc byte-identical to base on every slice; read=30 /
  quote=94 GNU-diff lines, unchanged from the qleak ledger (one earlier
  truncated read run was a host-load suite-timeout flake; three re-runs
  byte-stable).
- Pre-existing GNU divergences observed and NOT from this lane (both
  reproduce on the base binary): `:=` inside `$(( ))` does not persist
  the assignment (`echo "$((${n:=7}+1)) [$n]"` prints `8 []`, GNU
  `8 [7]` - arith-snapshot family); `$0` path-form in diagnostics
  (/d vs /mnt/d - environment).

### Leftovers

1. **10-pathmiss at 4.5x MSYS caliber**: first-fill 10.5ms (69 read_dirs
   on the host PATH - GNU pays 3 stats per miss on its 3-entry PATH) +
   the ~30ms harness parent constant. Natively 1.6x. Reaching 2x in the
   suite's own caliber is parent-bound.
2. **13/15 (and 04-07/16/20/24's exec half)**: owned by the per-command
   floor (arith_dyn snapshot, for-arith pair, buffered-echo path ~16us,
   strip-walker tail) - the ticketed deep subsystems; no single >5%
   site remains from this lane's vantage.
3. **11's ~130ms pipeline machinery** (native caliber): spawn/wire/wait
   of the internal yes|head pipeline - a pipeline-subsystem round.
4. **01/02 startup init ~11-16ms** vs GNU 5ms (native caliber): the
   in-process init path (locale, env import, PATH normalization).

## startup21 round (2026-09-30, wt21/startup21 on 0914200e): full 01/02 attribution + the exit-path cuts

The 2x-campaign's last bucket: 01/02 native-parent caliber (envfix3's
`scripts/run-perf-native-parent.py`, median of 15 unless noted). GNU anchors
re-measured inner-WSL same session: 01/02 best 4ms (script files).

### Full attribution (the 12-16ms, finally complete)

Scratch env-gated instrumentation (`RUBASH_STARTUP_PROFILE=1`, Instant phase
ticks + GetProcessTimes/GetSystemTimePreciseAsFileTime anchor + wall from the
native parent; module fully removed before commit). Probe 01, release,
steady state:

| segment | ms | owner |
|---|---:|---|
| process creation -> main entry (loader+CRT+static init) | 6.3-10 (swings with host load) | **HOST** |
| in-main work (see below) | 2.2-2.4 | rubash |
| Executor Drop (mailbox unregister + env restore) | 1.05-1.45 | rubash |
| post-Drop CRT exit + parent-side wait | ~0.3-0.5 | host/mixed |

**The pre-main segment is host-owned, not rubash's.** Evidence: a minimal
Rust exit-0 exe (105KB) measures the same anchor (interleaved A/B on one
load: minhello anchor med 9.0 vs rubash 6.8 — the ordering flips run to run,
so it is noise); `cmd.exe /c exit` measures 9.4-9.6ms median from the same
native parent; a 10MB image-padded exe (untouched .rdata) keeps the same
floor (min 9.5) — image SIZE is not the cost either. Whole-process CPU is
only 1.5-3ms: the pre-main wait is loader/AV wall time. perf10's unexplained
"7-9ms" was this segment, mis-attributed to the debug image.

In-main decomposition (before this round): mailbox register 0.48 / tools-dir
PATH probe 0.28 / env collect 0.12 + VarTable clone 0.08 / SHELLOPTS+BASHOPTS
replay 0.11 / thread spawn (512MB reserve) 0.15 / exec(`exit 0`) 0.32 /
tokenize 0.10 + parse 0.07 / fresh-env fs probes (PWD/THIS_SH/OLDPWD) 0.10 /
argv0 0.09 / struct+VariableStore 0.08 / prescans 0.07 / script read 0.05.
Drop: remove_file x2 0.36-0.50 + entry scan 0.06 / env diff 0.09 / ~60
putenv restore 0.34.

### Landed changes (7 files; instrumentation removed)

1. **Mailbox registration off the critical path** (`kill.rs
   register_signal_mailbox_async`): create_dir_all + marker write cost
   ~0.5ms per spawn; only OTHER processes' `kill` consults the marker (its
   content is never read — existence only). Registration is initiated at
   Executor::new (cf. GNU trap.c:102 initialize_signals ordering) on a
   background thread; Drop-ordered joins keep embedded executors exact.
   0.48 -> 0.05.
2. **Lazy POSIX-tools-dir probe** (`path.rs windows_posix_tools_dir`,
   `init.rs`): Executor::new no longer stats sh.exe/cat.exe/rm.exe across
   PATH entries (0.28ms); it records the startup shell-form PATH and the
   first consumer (`command -p`, logical /bin mapping) resolves it,
   memoized per PATH string. The probe is a pure function of the startup
   PATH string, so results are byte-equal to the eager probe, including
   the PATH-overwrite pin case (probe: `PATH=/bin:/usr/bin` then
   `command -p` still finds the toolset). 0.28 -> 0.00.
3. **Process-exit executor skips the Drop restore + unregister**
   (`mod.rs is_process_exit_executor`, `public_accessors.rs` Drop,
   `main.rs new_process_exit`): the env-restore (env diff + ~60 putenv,
   0.45ms) models an in-process child leaving its parent's process env
   untouched (rubash#182); a process that is about to exit has an
   unobservable environment. The exit-time mailbox unregister (two
   remove_file, 0.4ms) is replaced by deliver()'s existing dead-pid
   self-heal plus (4). Embedded/child executors keep both.
4. **Unregister enumerates with the pattern scan** (`kill.rs`): the
   read_dir-over-shared-dir loop became the FindFirstFileW pattern query
   the poller already uses (same files removed; the dir held 213 stale
   markers at measurement). Plus an opportunistic dead-pid prune (first
   128 entries) on the registration thread — off the critical path, keeps
   the shared dir bounded now that exit no longer removes own markers.
5. **Single env pass at process exit** (`init.rs new_inner`): the
   process_env_snapshot map (Drop-restore input, now skipped) is not
   cloned at startup for the exit executor. 0.08 -> 0.

### Numbers (release, alternating rounds, same session/load)

| probe | base 0914200e | startup21 | GNU inner | ratio before -> after |
|---|---:|---:|---:|---|
| 01-startup-empty | 11.3 | **9.5** (min 8.8) | 4 | 2.8x -> **2.4x** |
| 02-startup-fndef | 11.8 | **9.6** (min 9.1) | 4 | 3.0x -> **2.4x** |
| 10-pathmiss | 34.2 | 31.1 | 21 | 1.6x -> 1.5x |
| 11-pipeline | 138.4 | 135.1 | 7 | 19.8x -> 19.3x |
| 13-readloop | 150.9 | 142.6 | 23 | 6.6x -> 6.2x |
| 15-expansion | 531.0 | 532.9 | 57 | flat (noise) |

01/02 now sit AT the host process-creation floor (cmd.exe 9.4-9.6, minimal
exit-0 exe 9.1-9.5, same parent): rubash-owned in-process residual is
~1.3ms (exec 0.32 / fresh-env 0.3 / env import 0.2 / thread 0.15 /
tokenize+parse 0.17). First run after any rebuild pays a Defender re-scan
(+2-5ms on the first probe) — warm up before measuring.

### Observables changed (internal machinery, no GNU analog)

- `${__RUBASH_POSIX_TOOLS_DIR}` before first tools-dir use now holds the
  startup PATH string (the lazy source) instead of the resolved directory;
  consumers resolve to the same directory as before.
- The `{pid}.alive` marker appears ~0.5ms after process start (background
  thread) and is no longer removed at process exit (self-heal + prune).
- The process env is not restored at process exit (unobservable).

### Semantics gate (zero-change evidence)

- true-baseline slices (lane vs pristine-base 0914200e, both worktrees'
  harnesses, rb.out/rb.err/rc compared): **trap 0 GNU-diff lines, jobs 0**
  (GNU-identical on the lane), exp 0=0, new-exp 8=8, read 4=4, quote 0=0 —
  byte-identical lane-vs-base modulo pid and worktree-path noise. (Lane
  fixture initially lacked recho.exe/zecho.exe — copied from base before
  the run; the 380-line "regression" was that artifact.)
- GNU-diff probes (`target/issue-suites/results/startup21/probe*.sh`,
  script files, WSL 5.3.0 vs base vs lane): probe2 (in-process child env
  restore, rubash#182 family) GNU = base = lane byte-identical; probe3
  (`command -p`, before/after `PATH=/bin:/usr/bin`) identical modulo the
  platform path form. probe1/1b/1c (cross-process `kill -TERM` to a
  trapped child) show a PRE-EXISTING gap: GNU runs the trap (wait=7,
  TRAPPED) while BOTH base and lane hard-terminate (wait=143, NOTRAPPED)
  — byte-identical base-vs-lane; see leftovers.
- cargo test --lib 546/546, --test regression 27/27, RUSTFLAGS='-D
  warnings' cargo check --tests and --release --tests clean, cargo fmt
  --check clean, src/lexer/continuation.rs untouched, no stuck rubash
  processes at turn end.

### Leftovers (measured, with owners)

1. **The <2x target is bounded by the host floor in this caliber**: any
   Windows exe — including cmd.exe — measures 9.1-9.6ms from the native
   parent; GNU's 4ms is WSL/Linux process creation. Rubash's remaining
   in-process ~1.3ms is engine work with named owners (exec dispatch
   0.32 — perf11 per-command floor; tokenize+parse 0.17 — parse20;
   env import 0.2 — VarTable single-pass deep refactor; fresh-env fs
   probes 0.3 — each syscall ~40us). Cutting all of it lands ~8.5ms ≈
   2.1x — sub-2x needs the caliber re-hosted or the host floor itself.
2. **Cross-process TERM trap delivery gap** (probe1 family, pre-existing,
   base = lane): `kill -TERM` to a trapped rubash child hard-terminates
   it (signal_process fallback) instead of running the trap — both for a
   sleeping child (GNU: kernel wakes it) and a busy-loop child (the
   throttled 64-command file poll never observes the .q. entry in the
   probe window). The mailbox write itself works. Signal-subsystem round.
3. **Mailbox dir is ambient-dependent**: when an MSYS parent passes
   `TEMP=/tmp` unconverted, `std::env::temp_dir()` resolves
   drive-relative and the mailbox lands in `<cwd-drive>:\tmp\rubash-signals`
   (observed: D:\tmp dir created alongside the canonical
   C:\Users\...\Temp one). Pre-existing; also makes the dead-pid
   self-heal nondeterministic across parents (deliver and the marker can
   live in different dirs). A startup canonicalization of the mailbox
   dir would fix both; not this lane.
4. **deliver()'s dead-pid unregister is flaky on base AND lane** (multi-
   pid kills sometimes skip cleanup entirely - deliver never reaches the
   dead-pid branch in those runs; ambient per leftover 3). The lane's
   prune thread improves net cleanup either way.
## feeder21 round (2026-09-28, wt21/feeder21 on 0914200e): join/re-lex decomposition + battery fusion

The "feeder join/re-lex ~300ms" lane. The brief's hypothesis ("boundary
rejection still walks whole-line re-lex") was measured FIRST and found
already-gated; the real owners are per-line duplicated scanners and
bookkeeping. All numbers RELEASE build, alternating A/B against a pristine
0914200e base binary (private worktree target dir). Scratch instrumentation
(env-gated counters + Instant timers, fully removed before handoff).

### Decomposition (the brief's ask #1; instrumented, ~420ms wall)

The re-lex amplification is GONE on configure: total re-lexed volume =
577,845 chars full passes (18,429 resume-allowed-no-checkpoint, i.e. one
per committed logical line - each line IS the product) + 92,581 refused
(625 `}`-fold refusals at 1.7ms - the existing skip_brace probe already
gates this to fold-completions only) + 3,850 resume tails = ~1.03x the
file. The `}` refusal path the brief pointed at is 1.7ms, not 100ms.
configure -n gather (~182ms real) decomposes as:

| Piece | ms | Note |
| --- | ---: | --- |
| tokenize passes (tp_iterate) | ~45-50 | 19,054 passes; the lexer itself: word_finish 20-25 (quote removal walk + 3 String allocs/word), record_token 6.5, dispatch+ws ~17 |
| feeder 4 scans (comsub/quotes/compound/param) | 24.7 | tail-only per line |
| gather text battery (7 machines) | ~35 | quotes 4.2 + comsub 5.5 + subscript 4.5 + closechar 7.7 + fnbody 7.6 |
| heredoc_decls per line | 14.1 | full quote-aware walk + Vec<char> materialization |
| fold/gap-capture/heredoc_delims x2/funcheck | ~8 | per-pass token walks |
| aliases copy + paren_delta + group/pending/chars mirror pushes | ~25 | 24,753 per-line String clones + char collects |

Push-line branch exits: quotes-open 1,146 / continuation 272 / fastpath 23
/ comsub-rescan 14 / refused 625 / resume 78. Boundary checkpoints made
704, actually resumed 78 (configure's `}` lines refuse via the fold probe
- correctly).

### Landed changes (src/lexer/{mod,word,quotes}.rs + script_driver.rs; continuation.rs UNTOUCHED, git diff 0)

1. **Battery fusion (the round's core)**: `GroupScanFeeder` exposes
   `gates_prove_quotes_comsub_closed()` - the feeder's own quotes/comsub
   join-gate answers (closed AND un-parked) over the append-only
   `comsub_chars` mirror, with a sticky `group_clean` flag (dropped on
   backslash-continuation pop, IFS_GLUE insert, comsub-heredoc rotation,
   any heredoc-body line). When the group is clean, the gather's
   `GroupTextScans::needs_more` TAKES that answer for its duplicate
   quotes/comsub/balanced arms instead of re-advancing them: at a line
   terminator both machines from a closed, un-parked position are exactly
   `QuotesResidualState::default()` / closed (the '\n' arms only reset
   `comment_start`), so recording the default at the new mirror end is
   bit-identical to the machine's own advance; the comsub/balanced
   checkpoints stay where they were and re-advance on any later dirty
   line. Effect: ts_quotes 5,016 -> 1,107 advances, ts_comsub 4,139 -> 230.
   GNU anchor: parse.y:3557 read_token streams once; the two parallel
   scan batteries were this port's substitute for that model.
2. **Byte admissions**: `scan_heredoc_operators_full` returns early when
   the line has no `<` and no `(` byte and arith_depth==0 (every
   declaration arm needs `<`; only `(`/`$((` can raise the cross-line
   arith state) - heredoc_decls 14.1 -> 3.9ms. `line_paren_delta`
   returns 0 with no `(`/`)` byte.
3. **Per-line allocation trims**: expand_group_aliases skipped when the
   alias table is inert (was a String clone per line);
   pending_heredocs front checked by borrow (was a (String,bool) clone
   per line); `heredoc_delimiters` computed once per pass instead of
   twice (has_heredoc reuses the same Vec).
4. **Word-path trims** (finish_word_token): `is_assignment` computed once
   (was up to 3 walks); the non-`=(` raw passes through as `&str` (was a
   dead `raw.to_string()` before Token::new_with_raw's own copy);
   `remove_shell_quotes_inner` fast path - a word with none of
   `' " \ $ ` [ ]` de-quotes to itself (every mutating arm is keyed on
   those bytes; `pending_name` is armed only by `$`), so one byte scan +
   memcpy replaces the per-char state machine (word_finish ~25 -> ~20ms
   instrumented).

### Numbers (median of 10 alternating runs, steady state, clean build)

| Probe | base (0914200e) | lane | note |
| --- | ---: | ---: | --- |
| configure -n | 412 ms | 386 ms | -26 ms (-6%) |
| nvm -n | 225 ms | 216 ms | -9 ms |

configure -n rc=0 stdout/stderr byte-empty both sides; nvm -n outputs
byte-identical base-vs-lane. **The <250ms goal is NOT reachable from the
feeder family alone**: the remaining gather (~155ms) is ~50ms lexer
per-byte cost (word_finish quote-removal walk - GNU defers quote removal
to expansion, subst.c:4807 dequote_string runs at expand time, not lex
time; moving it out of the lexer is a token-architecture round), ~25ms
feeder scans + ~19ms battery subscript/closechar/fnbody (call-site
admission is NOT provable - the machines carry word/case_depth context
that arbitrary tails mutate), plus parse 144ms (parse20 bucket) and
~90ms startup floor (startup21 bucket).

### Zero-semantic-change evidence

- 546 lib + 27 regression + `RUSTFLAGS="-D warnings" cargo check
  --all-targets` + cargo fmt --check on the final clean tree.
- 14-shape 3-way matrix (`target/issue-suites/results/perf21/
  matrix-feeder21.sh` + m-{gnu,base,lane}-{n,exec}.{out,err,rc}):
  lane-vs-base byte-identical on ALL shapes/modes (clean multi-line
  quotes/comsubs, esac-paren lookaheads, backslash continuations,
  heredoc bodies with unbalanced quotes, $(cat <<EOF) rotation,
  assignment/bracket word shapes, fn signatures across lines, brace
  folds); GNU-vs-lane diffs identical to GNU-vs-base (pre-existing
  heredoc-in-dq-echo divergence + path prefixes).
- true-baseline.sh slices x10 (heredoc case comsub comsub2 quote errors
  cond func dstack redir): lane rb.out/rb.err byte-identical to base on
  every suite; GNU diff counts unchanged (quote 94, comsub2 67, cond
  105, comsub 27, heredoc 6, case 4, errors 4, func/dstack/redir 0).
- src/lexer/continuation.rs untouched (git diff 0 lines).

### Leftovers (measured, with owners)

1. **Lexer per-byte cost ~50ms on configure** (finish_word_token's eager
   quote removal + Token value/raw/leading_ws String allocs, ~71k
   tokens): GNU read_token stores the word once and defers dequote to
   expansion (subst.c:4807); the eager-dequote architecture is a
   token-model round (WordMetadata/carrier family).
2. **Battery subscript/closechar/fnbody ~19ms**: call-site byte
   admission unprovable (machines carry non-answer state arbitrary tails
   mutate); fusing them into the feeder needs machine-level rework in
   captain-exclusive continuation.rs.
3. **Feeder 4 scans ~25ms**: same admission blocker (comsub machine
   accumulates `word`/`case_depth` on every non-ws char).
4. **group Vec<(String,bool)> + exec_parts clones ~5ms**: borrow-thread
   refactor of run_history_group's data plane.

## quoterm22 round (2026-09-28, wt22/quoterm22 on a522eb39): word_finish quote-removal deferral evaluation + walk substrate

The "defer word_finish's eager quote removal" lane. The brief's two paths
were evaluated FIRST with fresh instrumentation; the measurement rejected
both as win-generators, and the round landed the low-risk substrate changes
that capture the actually-available win. All numbers RELEASE build,
interleaved A/B against a pristine a522eb39 base binary (env-gated counters
+ Instant timers, fully removed before handoff; instrumentation overhead
measured at ~2ms/bucket and stated separately).

### Fresh attribution (instrumented, configure -n, 40,163 words / 73,435 scan_token calls)

| Piece | instrumented ms | note |
| --- | ---: | --- |
| scan_token total | 40.6 | whole tokenize scan loop |
| word_finish value walk | 11.3 | fast path 28,911 words (72%, no `' " \ $ ` [ ]` byte) + slow walk 11,252 words / 282,396 chars |
| marker/kind arms | 5.8 | incl. ~1.5ms instrumentation overhead |
| Token::new_with_raw | 4.8 | 2 String copies per word (value copy + raw copy) |
| value == raw rate | 30,632/40,163 = 76.3% | dequote is identity for 3/4 of words |

word_finish total ~21.7ms instrumented (~17ms real) — NOT the ~50ms the
brief assumed; tokenize's other ~19ms (dispatch+ws, record_token, operator
Token::new) was feeder21's own leftover list, not word_finish.

### Path evaluation (the brief's ask #1)

- **(a) Token value 惰性化 (Cow/LazyCell): REJECTED.** 680 `.value` field
  reads across src (430 in parser/), Token derives Clone (LazyCell forces
  materialization on every clone), 1 write site, `pub value: String` is the
  parser/executor API surface. In the carrier family this is exactly the
  high-risk rewrite class; the win it could buy (skipping the value string
  when value==raw) is bounded by ~76% of words x one small-String alloc.
- **(b) defer to first consumer (word_value_from_raw pattern): REJECTED as
  a win.** Measured fact: on configure -n the FIRST consumer of essentially
  every word token's value is the parser itself (keyword compares + the
  AST `words: Vec<String>` clone), one step after lex. GNU's deferral
  target (expansion, subst.c:4807 dequote_string) is architecturally
  unavailable: rubash's AST/expander contract REQUIRES the lex-time
  carrier-laden value (DATA_DOLLAR/CTLESC family inserted during quote
  removal are consumed by the expander). Deferring to the parser boundary
  moves the walk, it does not remove it; net win = the double-copy only.
  That is the token-architecture round (WordMetadata/carrier family), not
  a zero-A/B-change lane.
- **Chosen low-risk road:** keep the eager dequote where it is (it IS the
  contract), but (1) stop paying a redundant copy, (2) make the walk
  span-copying. Both are byte-identical by construction.

### Landed changes (src/lexer/{quotes,word,token}.rs + scanner.rs; continuation.rs UNTOUCHED)

1. **Token::new_with_raw_owned (token.rs, finish_word_token + the scanner
   backtick site)**: the value String is MOVED into the token instead of
   copied — kills one of the three per-word String allocations.
   word_finish newtok bucket 4.8 -> 3.0ms instrumented.
2. **Byte-cursor span-copy walk (quotes.rs)**: `remove_shell_quotes_inner`
   (per-char `Peekable<Chars>` state machine) replaced by
   `remove_shell_quotes_inner_cursor` — a byte cursor over `&str` with
   bulk `push_str` spans between the ASCII trigger bytes, arm-for-arm
   transcription (main walk, `$(`/`${`/backtick/ANSI-C/double-quote
   subscanners as cursor twins; case-depth tracker logic unchanged, its
   esac lookahead now passes the remaining slice instead of collecting a
   fresh String). All trigger bytes are ASCII (UTF-8 continuation bytes
   are never triggers), `pending_name` can only arm at `$` and continue
   through [A-Za-z0-9_] — both facts recorded in the source header.
   GNU anchor: parse.y:5305 read_token_word / parse.y:5694-5706
   got_character carrier model unchanged; only the iteration substrate
   changed. The iterator-based subscanners remain verbatim for the rare
   remove_shell_quotes_outside_backticks path.
   One transcription bug was caught by the differential gate before any
   suite run: sub-cursors returning RELATIVE consumed counts were assigned
   to the caller's absolute index (`idx = f(&rest[idx..])` instead of
   `idx += ...`) — infinite re-push loop on `$(echo "\\")`; fixed to `+=`.
3. Walk bucket 11.3 -> 8.8ms instrumented (fast-path words unchanged).

### Numbers (interleaved A/B medians, clean builds, 16-20 runs)

| Probe | base (a522eb39) | lane | delta |
| --- | ---: | ---: | ---: |
| configure -n | 391-405 (median ~396) | 382.5 (stable across runs) | ~ -13 ms |
| nvm -n | 214-218 | 213.4-214.7 | ~ -2..-6 ms |

rc=0 both sides; stdout/stderr byte-identical base-vs-lane on both corpora.
**The <330ms goal is NOT reachable from the word_finish family**: fresh
instrumentation puts word_finish at ~17ms real of the ~390ms wall; the
remaining configure -n budget is parse ~144ms (parse20 bucket), startup
~90ms (startup21 bucket), tokenize non-word_finish ~19ms + feeder/battery
scans ~44ms (feeder21 leftovers, several needing captain-exclusive
continuation.rs). The brief's 50ms attribution for word_finish is disproven
by measurement; this lane took what the subsystem had.

### Zero-semantic-change evidence

- 546 lib tests + differential run (lane-local, removed before commit):
  cursor walk vs the original character-machine walk over every token AND
  whole line of all 737 files under third_party/bash/tests, nvm.sh, and
  bash's own configure — 354,153 tokens x posix on/off x assignment on/off,
  ZERO mismatches.
- **Full 83-suite true-baseline A/B** (the hard gate):
  `RUB_OVERRIDE=<base>` and `RUB_OVERRIDE=<lane>` full runs
  (`target/issue-suites/results/q22-tb-base` vs `q22-tb-lane`): rb.out,
  rb.err, and rb.rc byte-identical on ALL 83 suites; GNU-side ledgers
  identical (`target/quoterm22/tb-{base,lane}-ledger.log`).
- Post-cleanup spot re-verification (comments/test-module removal/fmt
  rebuild): configure -n and nvm -n byte-identical vs base; 6 quote-family
  suites (quote comsub comsub2 comsub-posix nquote new-exp) re-run
  byte-identical vs the gated base artifacts.
- `RUSTFLAGS="-D warnings" cargo check --all-targets` + `cargo fmt --check`
  clean on the final tree; src/lexer/continuation.rs untouched
  (`git diff a522eb39 -- src/lexer/continuation.rs` = 0).

### Leftovers (measured, with owners)

1. **Marker/kind arms ~5ms real**: quoted_literal_tilde + the Word-arm
   `is_assignment(&value) && assignment_value_is_quoted(raw)` pair. A
   conjunct reorder was tried and REVERTED (measured slower: '='-bearing
   non-assignment arguments then pay a full RHS quote walk instead of the
   2-char name-check exit). Needs a fused single-pass predicate, not a
   reorder.
2. **copy_braced_parameter_inner_cursor still allocates `wrapped` per
   `${`** (1,320 occurrences on configure): scan_braced_parameter's
   "&str starting at `$`" contract forces the copy; a view-taking scan
   is a dolbrace.rs round.
3. **Operator Token::new double-copies static text** (`"|"`, `"fi"` ... x2
   Strings each, ~31k non-word tokens): needs an inline-small-string Token
   representation — token-model round, paired with the value/raw unification.
4. **The remaining <330ms distance is not a word_finish problem** — see the
   budget note above; owners: parse20 (144ms), startup21 (~90ms),
   feeder21 leftovers (~44ms + 19ms tokenize non-word_finish).

## hist274 round (2026-10-01, wt23/hist274 on 2e6b0e98): arith_dyn lazy context + for-pair/errexit evaluations + f28 root-cause

The 2x-floor-bucket round (04/05) plus two evaluation-only deliverables and
the sig22 f28 follow-up. All numbers RELEASE, interleaved A/B vs a pristine
2e6b0e98 base built this session in ../rubash-wt-hist274-base; GNU anchors
re-measured by the suite in the same runs. Scratch instrumentation (per-name
snapshot timers, eval-phase timers, env-gated differential variants) fully
removed before the final gates; the final binary was re-gated from scratch.

### Task 1.1 — arith_dyn: design review first, then the GNU-faithful port

The brief asked for a "maintained entry" model (flip-point invalidation).
Design review REJECTED it: GNU maintains no eager entries for these names —
`initialize_dynamic_variables` (variables.c:1844) installs BASHPID,
BASH_SUBSHELL, BASH_ARGV0, BASH_COMMAND, GROUPS, SHELLOPTS, BASHOPTS,
PIPESTATUS, FUNCNAME as hash entries whose `dynamic_value` GETTERS run at
find_variable time (INIT_DYNAMIC_VAR, variables.c:1202; get_bash_command at
:1899 stores into the var only when read). An eager maintained entry for
BASH_COMMAND would flip on EVERY command (a new per-command String clone +
map insert taxing the write path rubash#156/#157 spent rounds removing), and
the 8+ scattered flip points each need independent semantic review
(execdeep's warning). The lazy port achieves GNU's cost model — zero for
names an expression never references — with a smaller surface.

Landed (src/executor/dynamic_arrays.rs ArithDynamicContext + arithmetic
{mod,parser,value}.rs + 7 construction sites): the per-evaluation 9-entry
HashMap snapshot (`Executor::arith_dynamic_values`, ~2.15µs/eval measured:
9 x dynamic_parameter_value + 9 String clones + with_capacity(9) alloc +
inserts) is replaced by a Copy-cheap context (bashpid/subshell_depth/
function_depth/pipestatus_first integers + funcname_top/debug_trap_command
Options that are None at top level) resolved per name at the point of use —
`resolve(env, name)` mirrors dynamic_parameter_value's arms exactly
(SHELLOPTS/BASHOPTS read the maintained env entry with the pure-env
recompute fallback; BASH_ARGV0 via the extracted env-only
script_name_value_from_env; GROUPS via the shared groups_words stub). parser
field `dynamic_values: Option<&ArithDynamicContext>`; consumption at
value.rs variable_value's existing precedence point.

Two same-family trims: the duplicated `array_expand_once` lookup hoisted
(assoc_noexpand reused on the identity-admitted path — provably unmutated;
the full pipeline re-looks-up because GNU computes tflag at expr_streval
time, post-expansion), and `arithmetic_last_eval_input` is now written only
on failure paths (GNU expr.c's expr_string/lasttp matter only once evalerror
records a diagnostic; all consumers — arithmetic_aliases:305,
embedded_mutations:1051/1160, parameter_core:260 — read post-None; the
nounset/bare-quote/empty-quoted early returns write it too).

### Numbers (interleaved A/B medians, same load window)

| probe | base | lane | delta | harness ratio (same-run GNU) |
|---|---:|---:|---:|---|
| 05-arith-x5000 (in-process) | 95.5 | 80.5 | **-15.7%** | 12.5x -> **10.7-11.2x** |
| 05 snapshot build alone (differential variant) | +21.5ms | 0 | matches instrumented 2.14µs x 10001 | — |
| 04-loop-true (in-process) | 117 | 113 | -3.4% | 11.9x -> 11.8x |
| 06/07 | flat | flat | noise band | — |

The 2x-floor goals (04 <10x, 05 <9x) are NOT reached: the snapshot's full
predicted value landed, but 05's remaining ~68ms in-process is the
distributed per-command floor — see leftovers.

### Gates

- matrices m1 (the nine dynamic names x every arith entry kind, subshell
  depths, unset marks, options on/off, subscript contexts),
  m2 (21 arithmetic FAILURE forms whose diagnostics read the deferred input
  slot), m3 (array_expand_once, assign-to-dynamic, subscript side effects):
  base-vs-lane byte-identical stdout+stderr+rc (`target/hist274/out/`).
  Lane-vs-GNU deltas are pre-existing and base-identical: `((BASH_SUBSHELL =
  5))` does not persist (GNU assign_subshell sets subshell_level, rubash
  writes env only) — semantic ticket class, unchanged by this lane.
- true-baseline slices (arith arith-for array assoc new-exp exp): base and
  final lane byte-identical rb.out/rb.err/rb.rc; GNU diff counts identical
  (0/0/8/46/4/0). **Harness pitfall recorded**: three earlier suite runs
  silently produced EMPTY rubash output (0-byte rb.out on BOTH binaries) and
  the ledger printed 301/90/777/385/912 "diff lines" — those were
  empty-vs-GNU artifacts of a broken fixture state, not real baselines;
  a 0-byte rb.out in this harness means the run is void, re-run before
  reading the ledger.
- 546/546 lib + 27/27 regression + `RUSTFLAGS='-D warnings' cargo check
  --tests` and `--release --tests` clean + cargo fmt --check clean;
  src/lexer/continuation.rs untouched (git diff 0).

### Task 1.2 — for ((...)) pair: memo REJECTED by decomposition

Differential micro-probes (20k iterations, timeout-guarded; one rubash.exe
pair was left running by a bad probe and killed before finish):
`(( i += 1 ))` command = 3.3µs end-to-end, `:` = 6.5µs, assignment =
7.75µs; for-form test/update eval = 3.9µs/eval vs GNU 0.6µs (f1 shape GNU
40ms vs lane 279ms = 7.0x). Real loop test/update expressions CONTAIN the
loop variable — a constant-expression memo (GNU has none; execute_cmd.c:3201
eval_arith_for_expr -> evalexp every iteration) would not apply to the
shapes that matter. The pair cost decomposes as: eval pipeline ~1.5µs
(parser 0.95 + tail 0.37) + restore_for_line ~0.4µs (an env get+insert with
two String allocs per expression; GNU execute_cmd.c:3236 assigns an int) +
xtrace/debug gates ~0.25µs. Owners: the eval-pipeline floor (already the
per-command-floor family) and the __RUBASH_CURRENT_LINE-in-env model (a
Cell<usize> port with env sync only at consumers — broad surface, its own
round).

### Task 1.3 — errexit/xtrace read convergence: evaluated, NOT landed

Reads: errexit_enabled() x15 sites, xtrace_enabled() x16 sites; each read =
1-2 env lookups (~0.1-0.15µs) consulting TWO encodings (__RUBASH_ERREXIT
live marker, then shell_option_enabled's __RUBASH_SETOPT_* attr). Writers
(~9 sites): set.rs apply_short_set_flag 'e'/'x' (writes BOTH encodings),
the long form via set_shell_option, Executor::new SHELLOPTS replay,
command_substitution.rs:1193/1474 (marker REMOVE only — the comsub child
disables errexit by dropping the live marker while the option attr stays
set, so inside a comsub child errexit_enabled() consults marker-then-attr
and can return TRUE via the attr; whether that matches GNU's
execute_cmd.c:1669 subshell reset is unverified), embedded_mutations
save/restore block, shell_options.rs:651. Measured stake: 2-3 reads per
command on the hot paths (~0.3-0.45µs) = 3-4.5ms on probe 05 (~5%).
Convergence design (env stays authoritative; ShellState Cell<bool> read
cache recomputed by a resync helper at every writer + carried by ShellState
clone) is sound but the dual-encoding comsub corner above must be settled
against GNU first — exactly the "8+ flip points, independent review"
surface execdeep flagged. Left for the flag-convergence round with the
OptionTable/VarTable owners.

### Task 2 — f28 (sig22 follow-up): root-caused to an fd bug, NOT xtrace; issue #368 filed

The nvm fast test "Running 'nvm-exec' should display required node version"
fails on the pristine base at its first capture — but the capture corruption
is fd-shape, not xtrace inheritance. Minimal repro (issue #368):
`{ c="$(echo progress; echo value >&3)"; } 3>&1 1>&4; echo "[$c]"` — GNU
prints `progress` outside and captures `[value]`; rubash captures BOTH
(pre-opened fds 3/4 are invisible inside $( ); `>&3` and `1>&4` silently
fall back to the capture pipe). nvm.sh's nvm_rc_version ends `nvm_echo
"$VER" >&3` and nvm-exec consumes it with `{ V="$(nvm_rc_version 3>&1 1>&4)";
} 4>&1`, so the version capture collapses and the whole error cascade
(including the observed `0;31m`/`256` color-code lines inside the capture)
follows. xtrace verification recorded in the issue: GNU never auto-exports
SHELLOPTS (variables.c:510 readonly import; shell.c:1971-1987 children
import from env only when non-privileged) and no xtrace inheritance could
be reproduced on base or lane in any child shape (external, shebang ->
rubash, `rubash -c`, ambient env) — the sig22 "inherited into capture"
symptom is this fd bug.

### Task 3 — battery three-machine redo: EVALUATION (captain decision input)

feeder21's leftover: gather battery subscript 4.5ms + closechar 7.7ms +
fnbody 7.6ms (~19.8ms on configure -n). State classification (verified in
the oracles, not assumed):

- **ANSWER state** (may not be dropped at a boundary): subscript's
  `reported` + its quote/comment scan bits (single/double/ansi_single/
  escaped/in_comment) and command_position/compassign_depth/element_start;
  closechar's `stack` + case_depth/case_in_stage/word_boundary
  (the `)`-inside-case disambiguation, parse.y:1037); fnbody's
  phase/depth/search_from + CommentAwareScan.
- **NON-ANSWER state** (full-String bookkeeping where only a classification
  is ever read — the fusion blocker feeder21 named): subscript's `word` is
  consumed ONLY by `word.is_empty()`, `is_pure_identifier(&word)` and
  `is_command_position_boundary(&word)` (skip.rs `[` arm + newline arm) —
  a 3-state summary (Empty/PureIdent/Other + reserved-word tracker) drives
  identical answers. closechar's `cur_word` is read only as reserved-set
  membership (`"if"|"then"|...|"case"`), `ends_with('=')`, `len()>1`,
  is_empty (continuation.rs:493-511,670); its case-tracker `word` is read
  only as `word == "esac"` — a prefix-progress byte suffices. These two
  Strings make every per-line checkpoint snapshot a clone and pay a
  per-alnum-char push on every tail advance.

Redo sketch (two independent stages, both touch captain-exclusive
continuation.rs): (1) summary-state port of subscript/closechar `word`
fields (differential-gated vs the String oracle over the full corpus,
quoterm22-style); (2) extend feeder21's fusion pattern —
`needs_more(skip_quote_comsub)` gains feeder-maintained boundary summaries
for the three machines' ANSWER state so clean lines take the answer instead
of re-advancing (fnbody's rare signature check keeps its `pending` read at
the delimiter transition only). Expected: stage 1 ~5-7ms (clone+push
elimination), stage 2 net ~8-12ms more (feeder pays a small per-char update)
— combined ~13-19ms of configure -n's ~386ms (-3.5-5%), same share on
nvm -n. NOT implemented: both stages edit continuation.rs machines and the
oracle equivalence proofs are the captain's gate.

## perf1x round (2026-10-01, wt30/perf1x on 49057268): errexit/xtrace single-source convergence

The hist274 bucket-3 handoff (31 read points x 1-2 env queries, ~5% of
probe 05) with its prerequisite settled first. Two commits, both
correctness-first; the perf delta is honestly below the noise floor.

### Task 0 (the prerequisite): GNU's comsub errexit entry, verified

GNU model (source + WSL 5.3.0 script-file probes `target/perf1x/e*.sh`):
`command_substitute`'s fork child runs `builtin_ignoring_errexit = 0;
change_flag ('e', FLAG_OFF); set_shellopts();` (subst.c:7356-7362) — the
-e OPTION itself is cleared (flags.c:171 zeroes errexit_flag;
exit_immediately_on_error follows at flags.c:261-263), so inside the body
`$-` shows no 'e', `$SHELLOPTS` has no errexit, and an explicit `set -e`
re-enables it. The nofork funsub does the same (subst.c:7020-7029). Two
of rubash's four comsub entries modeled this; two did not — live
divergences at 49057268:

1. `run_ast_command_substitution_with_context` suppressed errexit with
   the counter only: `$-`/`$SHELLOPTS` inside the body showed errexit
   (probes e6/e7; GNU shows neither).
2. `run_function_command_substitution` (the `$(f)` shortcut) had NO
   adjustment: `set -e; f() { false; echo inner; }; echo $(f)` captured
   empty (GNU captures `inner`; probes e3/e4e).

**Commit 7002db37**: both entries now apply the GNU adjustment bracketed
for in-place execution (counter save+reset, option off; env pair rolled
back by restore_flat_subshell, counter restored after the capture). All
four divergent shapes byte-identical to GNU; every previously-matching
shape unchanged (25-case matrix `target/perf1x/m1.sh`).

### Task 1: the markers deleted, option table is the single source

The dual encoding itself was broken: the live markers
(`__RUBASH_ERREXIT`/`__RUBASH_XTRACE`) were written ONLY by the short
`-e`/`-x` form while the long form wrote just the option attr, and the
read was `marker || attr` — any stale marker resurrected the flag:

- `set -e; set +o errexit; false` -> rubash exited, GNU survives (d1/d3)
- `set -x; set +o xtrace; echo end` -> rubash traced it, GNU does not (d4)
- `set -x; ...; set +x; echo end1` -> base traced end1 (m2 matrix)

GNU has ONE flag variable per option (flags.c:56-57/171, change_flag
flags.c:226). **Commit 74a06435** deletes the markers everywhere (set.rs
short form, the four comsub adjustments, funsub save/restore,
shell_options `set -` operand, compound_exec spawn-inheritance list);
`errexit_enabled()` (15 sites) / `xtrace_enabled()` (16 sites) read the
option table alone — one env lookup per read instead of two, and the
stale-marker class is structurally gone. The xtrace x comsub matrix
(m2) is byte-identical to GNU including `++` indirection levels.

### A/B (release, interleaved 8 rounds, load-amplified 20x per niubash#155)

| probe (amplified) | base median | lane median | delta |
|---|---:|---:|---:|
| arith loop 100000 iters (in-process) | 977.7 ms | 971.7 ms | -0.6% (noise band ±5%) |
| `:` loop 50000 iters (in-process) | 2003.0 ms | 2002.7 ms | flat |

The hist274 estimate ("~5% of probe 05") is NOT confirmed at the wall
level: even at 20x amplification the saved lookup is below the host
noise floor. The commits stand on the GNU-alignment fixes (three fixed
divergence families); the read convergence's remaining value is the
eliminated desync surface, not wall time. A ShellState Cell cache on top
(single attr lookup -> field read) is now unnecessary for correctness
and sub-noise for perf — do not spend a round on it.

### Gates

549/549 lib + 27/27 regression (both commits); RUSTFLAGS='-D warnings'
cargo check --all-targets clean; cargo fmt clean; true-baseline slices:
set-e comsub comsub2 trap dstack 0 diff lines; errors/read/exp
base-vs-lane byte-identical modulo each binary's own $0 path (remaining
GNU diffs pre-existing); canaries yes|head 100000 and seq 5000000|wc -l
pass; `$(seq 20000 | cat)` hangs identically on base (pre-existing
#158, wt27 lane's); continuation.rs untouched.

### Side findings (pre-existing, recorded with reproducers)

1. **posix-mode stickiness gap**: GNU's `set -o posix` turns shopt
   inherit_errexit ON and `set +o posix` leaves it ON (sticky —
   `target/perf1x/m1y.sh`: GNU off/on/on, rubash off/off/off). Visible
   in comsub errexit behavior after a posix round-trip (m1 cases
   22/24/25). Owner: the posix flip site, separate subsystem.
2. **Pre-existing nested-comsub/EXIT-trap shapes** (m1x) match GNU in
   isolation; the m1 22/24/25 diffs are entirely finding 1.

## perf2b round (2026-10-02, wt34/perf2 on 4928cfa6): CURRENT_LINE single
author, bind_underscore gates, and the parse/feeder bucket decomposition

Three-bucket round (hist274 handoff). All wall numbers RELEASE build,
native-Python-parent interleaved A/B medians against a pristine 4928cfa6
binary built this session (`target/rubash-base-wt34.exe`, same load
window); GNU anchors re-measured inner-WSL this session (p-null 37ms,
p-f1 172ms, nvm -n ~17.9ms, wordfor 5.5ms). Scratch instrumentation (the
`perf34_scratch` parse/feeder profiler) fully removed before commit; the
final clean binary was re-gated from scratch.

### Bucket 1 disposition: Cell-ization EVALUATED, single gated author
landed instead (310fc49f)

The Cell design (env authority -> `Cell<usize>` + consumer sync) was
rejected on architecture evidence, not perf-noise grounds:

- 62 in-process `__RUBASH_CURRENT_LINE` read sites; ~16 builtin
  diagnostic-prefix helpers receive ONLY `&VarTable` (builtins take
  `args + &mut VarTable`, never the Executor), so a Cell authority needs
  either publish-at-dispatch churn across every per-builtin dispatch
  site (printf_path_builtins.rs alone has ~15 cd entry points) or a
  second readable encoding — exactly the desync surface wt30/perf1x
  deleted for errexit/xtrace markers.
- The measured win was in the WRITE path, not the read path:
  `restore_for_line` (execute_arithmetic_for_command) and the word-list
  for restore (loop_select) did UNCONDITIONAL
  `insert(key.to_string(), line.clone())` — two String allocations + a
  map insert per restore, twice per arith-for iteration where GNU's
  execute_cmd.c:3236 restore is an int store. The per-command stamps
  were already equality-gated (perf11).

Landed: ONE gated entry point (`Executor::set_current_line_value`,
stack-rendered, equality-gated insert) now authors the env entry — every
writer funnels through it (set_current_line, the for-arith/word-list
restores with parsed `Option<usize>` captures instead of Strings — the
per-iteration ambient-line parse also disappears —, the reader-loop
stamp, select/function/debug/function-exit pins, and source's restore
bracket). Single source kept: the env entry has exactly one author, no
second encoding.

### Bucket 2: bind_underscore no-op gates (dc7bef8e) + eval-pipeline
evaluation (nothing landed)

- bind_underscore: GNU bind_lastarg (execute_cmd.c:4191) rebinds the
  cell but only the VALUE is observable; the equality gates skip the
  env-mirror insert and typed-store clone when unchanged (`:` in a loop
  rebinds the same word thousands of times). perf17's 1.8us residual
  becomes ~two map gets for that shape.
- Eval pipeline (~1.5us/eval, hist274): the "extra vs GNU" items are
  (a) three unconditional env-marker removes per evaluation
  (`__RUBASH_ARITH_SUBSCRIPT_EXPR` / `_EXP_EXPANDED` / `_READONLY_ERROR`
  — inserted by parser-internal error paths, removed per eval; a Cell
  port needs threading a Cell handle through
  eval_mutable_arith_value*'s parser, and the markers have NESTED-eval
  clearing semantics that must be reproduced exactly), (b) the nounset
  option HashMap read (GNU tests `unbound_variable_is_error`; the fix is
  the option-table flag model perf1x evaluated as sub-noise), (c)
  `record_arith_write` (rollback semantics — semantic, not waste). Each
  is 40-160ns/eval; none crosses the honesty bar for this round. The
  instrumented split (p-f1, RUBASH_EXEC_PROFILE): for_body 46% of wall,
  eval pair 17% (test 1.3us + update 2.2us — the delta is the `i++`
  write-back: bash_arith render + env insert + ARITH_WRITES log +
  process-env sync gate), linecmd/chain/scans ~7%.

### Bucket 3: fresh nvm -n decomposition + design section (no landing)

Scratch phase profiler over nvm -n (172,906 bytes, 5732 lines, 15,048
tokens, 8,549 CommandNode::new, wall ~184ms release): lextok 78-81ms
(ONE whole-file tokenize call — the non-alias driver path; the gather
family timers all read 0), parse 212-253ms of which body-reparse
91-108ms (358 calls — parse20's if-inline left case/for/fn bodies),
word-intake 27-35ms (WordScans 17-25ms: parameter scan 9.9 + comsub 4.2
+ quotes 3.0 instrumented; store clones 4ms), main-loop remainder ~90ms
(try_parse_compound_start is 96% nested-INCLUSIVE — it does not localize
anything). configure -n now ABORTS at line 23359 (`esac |` — GNU rc=0)
identically on base and lane: a PRE-EXISTING master parse gap (the wt29
#380 family residue), so nvm is the clean corpus.

**Design section (the architecture item, for a future round): relocate
per-word parse-time scans to expansion time.** GNU runs NO per-word
expansion scans at parse (parse.y:5305 read_token_word builds the word
once; make_cmd.c stores the WORD_DESC; subst.c analyzes at execution).
Rubash's 9 `*_in_word` scans build the executor-facing metadata
(comsubs/params/braces/quotes/extglob/tilde/pathname/procsub) at parse
time — ~17-25ms of nvm -n's 184ms and the same share of every parse —
plus the triple word store (token.value + cmd.words[i] +
metadata.value/.raw — 3 clones/word). Staged plan: (1) make
WordMetadata fields `Rc`-shared with the token stream so the store
clones become refcount bumps (84 reader sites, mechanical but wide);
(2) move scans 1-9 behind a lazy WordMetadata accessor consumed by the
executor's expansion preamble (identical outputs — the scans are pure
functions of (value, raw); the parse-time callers that need them today
are the error-owning pre-scan family perf6 gated, which stays);
(3) only then consider fusing the three `$`-triggered walkers
(comsub/param/arith walk the same `$(`/`${` spans independently).
Guardrail: the quoterm22-style differential over the full corpus before
each stage. NOT attempted this round (stage 2 changes when errors fire
relative to execution — needs the perf6 per-arm lazy-error analysis).

### Numbers (final clean binary, interleaved A/B vs pristine 4928cfa6)

| probe | base ms | lane ms | delta | GNU ms | ratio |
|---|---:|---:|---:|---:|---:|
| p-null (20000-iter `for ((;;)) :`) | 232.6 | 221.6 | -4.7% | 37 | 6.3x -> 6.0x |
| p-f1 (100000-iter) | 1100.4 | 1038.1 | -5.7% | 172 | 6.4x -> 6.0x |
| p-wordfor (900 word-list iterations) | 13.8 | 13.3 | -3.3% | 5.5 | 2.5x -> 2.4x |
| nvm -n | 183.9 | 184.0 | flat | 17.9 | 10.3x (parse-bound) |
| configure -n (aborts :23359 both) | 346.1 | 342.9 | -0.9% | — | pre-existing gap |

Per-commit: 310fc49f alone measured p-null -2.9% / p-f1 -4.1%; dc7bef8e
stacks to the combined -4.7% / -5.7%.

### Gates

- lib 553/553; regression 27/27; issue354 carrier 9/9; `RUSTFLAGS='-D
  warnings' cargo check --all-targets` clean; cargo fmt --check clean;
  continuation.rs untouched (never edited); tree byte-clean vs
  dc7bef8e after scratch removal.
- true-baseline slices (errors read arith arith-for func trap dstack
  quote) on the final debug binary: base-vs-lane byte-identical (rb.out
  identical everywhere; errors/rb.err differs ONLY in each binary's own
  $0 path); GNU ledger counts identical (errors 2, read 2, rest 0).
- canaries on the final release binary, base-identical: #380-shape
  `$(case x in case) echo hi;; esac)` -> `[]` == GNU; `$(seq 20000|cat)`
  completes under `timeout 10` (len=108893); `yes|head -100000` ->
  100000; `seq 5000000|wc -l` x3 full (5000000). Underscore matrix
  (loop/word binds, readonly `_`, set -a window, `$LINENO`) base-vs-lane
  byte-identical. No stuck rubash/bash suite processes at turn end.

## parse4 round (2026-10-03, wt44/parse4 on 69f0934c): the scan-relocation
design section executed (phases 1-2) + store/body-reparse evaluation

Sixth attack at #241, executing perf2b's design section ("relocate
per-word parse-time scans to expansion time"). Two commits, one tree
(wt44/parse4), all numbers RELEASE, interleaved A/B medians vs binaries
built this session: pristine master 69f0934c (`rubash-base-wt44.exe`),
phase-1 HEAD 4f5f7b23 (`rubash-c1-wt44.exe`), phase-2 HEAD 44f7227a
(`target/release/rubash.exe`). GNU anchor re-measured inner-WSL from a
tmpfs copy (nvm -n median 18.5 ms over 5 runs).

### Phase 1 (4f5f7b23, predecessor): zero-reader scans deleted

Parameter / arithmetic / brace / tilde parse-time word scans and their
record_* wrappers (~1300 lines): the store vectors had ZERO readers
(field-syntax audit across src/); the executor re-derives all four from
the word text at expansion time — GNU parse.y:5305 read_token_word
builds the word once, make_cmd.c stores the WORD_DESC once, subst.c
analyzes at use.

### Phase 2 (44f7227a, this continuation): comsub + word-quote scans
move to their consumers

The remaining two parse-time word scans had only re-derivable readers:
cmd/WordMetadata.command_substitutions fed only the pretty-print
renderer (ast_print render_word) and the bare-`((` admission
(arithmetic_command_is_bare); cmd/WordMetadata.word_quotes fed only the
conditional quoted-RHS check. Both now re-derive on demand via
command_substitutions_in_word_public / word_quotes_in_raw_public — the
same pure scans over the same inputs (identical span boundaries; the
former is print_comsub's oracle per rubash#274). The bare-`((`
admission becomes a whitelist (every comsub needs a `$`/backtick byte
in a word or assignment value — exactly the surfaces the old store
covered). The LIVE word_quotes stores (ArrayElementAssignment /
CompoundAssignmentElement, read by array_assignment_exec's
syntactic-compound-list decision) stay.

### Numbers (interleaved A/B medians)

| probe | master -> p1 | p1 -> p2 | master -> p2 | GNU anchor |
|---|---:|---:|---:|---:|
| nvm -n | 315.4 -> 284.5 (-9.8%) | 278.0 -> 237.6 (-14.5%) | 315.4 -> 230.8 (**-26.6%**) | 18.5 ms -> 17.0x -> 12.7x |
| nvm load | 409.8 -> 376.3 (-8.2%) | 373.8 -> 322.8 (-13.7%) | 411.8 -> 323.0 (**-21.6%**) | — |
| configure-head1374 -n | 1174.8 -> 1094.9 (-6.8%) | 1117.5 -> 1081.7 (-3.2%) | (same-window pairs) | — |
| p-null 20000-iter | — | — | 200.1 -> 199.1 (-0.5%, noise) | — |

Final clean-binary re-verification after instrumentation removal: nvm
-n -24.7% vs master; canaries 12/12.

### Gates (phase 2, final tree)

- 535-file -n corpus differential master-vs-lane: 0 differ, 0 timeout
  (`target/issue-suites/results/parse4/c3-corpus/`).
- true-baseline 18-suite slice (exp new-exp more-exp posixexp array
  assoc comsub comsub2 quote braces extglob case cond errors read
  dstack heredoc redir): rb.out/rb.rc byte-identical; rb.err
  byte-identical modulo each binary's own $0 path (errors/redir only,
  0 diff lines beyond the path). RUB_OVERRIDE must be a /mnt/... path —
  a `D:\...` path makes WSL timeout fail to exec the exe and the rb
  side goes vacuously empty (caught; artifacts
  `c3-tb-base`/`c3-tb-lane` are the valid rerun).
- Canary batteries 21/21 byte-identical: the 12-case predecessor
  battery (#380/#414 shapes, seq 20000|cat, seq 5000000, yes|head,
  wordfor, compound/array, tilde, arith) + 9 new consumer-shaped cases
  (pretty-print comsub-dense/heredoc-comsub/nested-backtick,
  declare -f comsub/backtick, quoted-RHS =~ matrix, bare-`((`
  mixed/comsub-var, assignment-position comsub) — the -n corpus does
  not cover these consumer paths.
- lib 560/560; parser_tests 316/316; full cargo test --no-fail-fast
  failure set identical to master (5 pre-existing: 3 bashdb fixture +
  invalid_cli_shopt + parameter_transform flake); cargo fmt --check
  clean; continuation.rs untouched.

### Store triple-clone evaluation (quoterm22 discipline: measure first)

Fresh instrumentation (env-gated hierarchical timers, removed before
handoff): store_self = **3.1-3.3 ms** on nvm -n (6,723 words, ~470
ns/word; word_intake minus scans and store is 1.0 ms, the remaining
extglob/pathname/procsub scan trio 0.8 ms) and **0.2-0.3 ms** on
configure-head1374 -n (the posix driver's parse phase there is only
~4.5 ms of a ~1080 ms wall — the grouped-driver scans dominate, not
parse). Ceiling ~1.3% of the nvm -n wall. The only real implementation
path is Token.value/raw -> Rc<str> PLUS WordMetadata.value/raw ->
Rc<str> PLUS cmd.words -> Vec<Rc<str>> in one change (parse holds
tokens as &[Token], so with String fields every store copy MUST be a
fresh allocation; a 2-of-3 share cannot be constructed). That is the
carrier-family-wide token-architecture round (quoterm22 leftover #3),
not a standalone zero-A/B-change lane. **REJECTED this round — below
the honesty bar** (quoterm22 precedent); recorded for the
token-architecture round.

### Body-reparse cache feasibility: REJECTED (no reuse exists)

Fresh numbers: nvm -n runs 481 nested body parses (62.8 ms inclusive
sum, nesting double-counted; the whole parse phase is 29.2 ms of the
238 ms wall), configure-head 179 / ~5 ms. Every body is parsed exactly
once: function bodies parse once at definition and execute_function
reuses the stored AST per call (clones it, never re-parses); each
case/for/loop/brace/subshell body occurs once per script position. A
text-keyed cache has zero expected hits — the 91-108 ms perf2b saw was
the two-pass architecture (scan-for-closer + recursive parse), not
repetition. Load-mode comsub bodies DO re-parse per evaluation, but
that is GNU parity (subst.c:7143 command_substitute ->
parse_and_execute per evaluation). The structural fix is extending
parse20's inline-section mechanism (if-family is already single-pass
in run_command_loop) to case/for/loop/brace/subshell/function — six
parser integrations, deep subsystem, its own round.
