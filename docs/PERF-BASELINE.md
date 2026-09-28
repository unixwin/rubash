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
