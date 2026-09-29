# Bash Corpus Coverage Inventory (single source of truth for "what has been run")

> Created 2026-09-27 by the corpus-discovery lane (wt4/corpus2, rubash `9bf2df9e`).
> Purpose: prevent re-running corpora that earlier waves already measured, and record
> the exact evidence (issues, artifacts, reports) for every corpus.
>
> Rule for any future lane: **check this file before running anything.** A corpus is
> "covered" if it has a row here with a run date and an issue/artifact reference.
> Verify against this inventory, not against memory — several candidates that "felt new"
> had already been run in `target-ecosys*` / `target-tools` waves (2026-09-26/27).
>
> Evidence sources mined: `gh issue list --repo unixwin/rubash --state all` (all 175+
> issues, #15–#213), per-wave `REPORT.md` files in the `target-ecosys*` / `target-tools`
> sandboxes (git-ignored, live under `D:/repo/rubash/`), `docs/*.md`, and the artifact
> trees under `D:/repo/rubash/target/issue-suites/results/` (432 entries).

## 1. Prior waves (evidence-mined; do NOT re-run)

| Corpus | Source / version | When run | Verdict | Issues filed | Artifacts |
| --- | --- | --- | --- | --- | --- |
| GNU 83 test suite (all `third_party/bash/tests/*.tests`) | bash.git @b4608166 (Bash-5.3 p15) | continuous; ledger 2026-09-25 (`docs/COMPATIBILITY-STATUS.md`) | 82 zero-diff / 1 residual (nameref timing) | #25, G1–G26 (#72–#98), many later | `target/issue-suites/results/true-baseline*` (present) |
| suite2 re-run (20 GNU suites, 180s timeout) | same | 2026-09-27 | consistent with 09-25 ledger | none new | `D:/repo/rubash/target-suite2/` (present) |
| oils spec / mksh check.t / ksh93 / busybox ash | upstream trees | 2026-08..09 | large diff batches, engine gaps | #20–#24, #37–#57 | `target/issue-suites/results/oils-*`, `mksh`, busybox logs (partially present) |
| linux-smoke (true GNU binaries on Linux) | self-built | 2026-09 | #138–#140 family evidence | #138 #139 #140 | `target/issue-suites/results/linux-smoke/` (present) |
| bash-completion 2.18.0 (core + 452 completions-core files + 11 driven) | master 032805e | 2026-09-26 (ecosys1) + 2026-09-26 (ecosys3) | load-dead → partially fixed; run-face parity except compopt | #138 #144 #146 #147 #153 | `target-ecosys/`, `target-ecosys3/` (present) |
| git-completion.bash | official | 2026-09-26 | zsh-style `${(M)...}` parse rejection (fixed since) | #142 | `target-ecosys/` |
| oh-my-bash (full repo, 234 files) + 5-theme matrix + full init | master (MIT) | 2026-09-26 (ecosys1/2/3) | `-n` A-level regressions ×2; init broken by glob/eval family | #143 #148 #149 #160 | `target-ecosys{,2,3}/` |
| bash-it (full repo, 343 files) + full load + composure | master (MIT) | 2026-09-26 (ecosys1/2/3) | full-load broken (comsub leak family) | #161 | `target-ecosys{,2,3}/` |
| nvm.sh | master, 5226–5228 lines | 2026-09-26/27 (ecosys3/5) | perf O(N²) + File-name-too-long + `nvm ls` jobs family | #130 #155 #162 #169 | `target-ecosys{3,5}/` |
| rustup-init.sh | rustup master, 930 lines | 2026-09-26/27 (ecosys3 + target-tools A) | `--help` false-warn (grep argv quotes) | #205 | `target-ecosys3/`, `target-tools/results/A-rustup` |
| rbenv (init chain, commands) + jenv (same architecture) | master 07e9b1e | 2026-09-26/27 (ecosys5) | 127-cascade + $0 self-location + PATH colon | #166 #171 #184 #191 #193 | `target-ecosys{3,5}/` |
| ruby-build | master | 2026-09-26/27 (ecosys3/5) | --help hang, $0 family | #171 #185 | `target-ecosys{3,5}/` |
| pyenv 2.8.6 + python-build + pyenv-virtualenv (full CLI drive) | master | 2026-09-27 (ecosys5) | 127-cascade (#154+#171) — **pyenv IS covered** | (via #154 #171 cascade) | `target-ecosys5/` |
| conda.sh (activate/deactivate shim, `_CONDA_EXE` stubbed) | conda master, 74-line shim | 2026-09-27 (ecosys5) | covered | (in ecosys5 REPORT) | `target-ecosys5/` |
| sdkman, gvm, asdf v0.15.0 (last bash version), volta (no bash) | upstream tags | 2026-09-27 (ecosys5) | covered / recorded | (in ecosys5 REPORT; gvm in #171 body) | `target-ecosys5/` |
| starship init template (starship.bash) | master (ISC) | 2026-09-26/27 (target-tools C) | declare -f re-serialization, 71 diff lines | #202 | `target-tools/results/C-starship` |
| fzf shell/key-bindings.bash + shell/completion.bash | master (MIT) | 2026-09-26/27 (target-tools C) | **clean** (0 semantic diff; function defs, bindings, errors identical) | none | `target-tools/results/C-fzf-*` |
| atuin.bash (34KB) | master | 2026-09-26/27 (target-tools C) | #202 family, 393 lines | #202 | `target-tools/results/C-atuin` |
| direnv stdlib.sh (41KB) | master | 2026-09-26/27 (target-tools C) | #202 family, 701 lines | #202 | `target-tools/results/C-direnv-stdlib` |
| zoxide template | master | 2026-09-26/27 (target-tools C) | #203 family, 24 lines | #203 (family) | `target-tools/results/C-zoxide` |
| installers: get-docker.sh, deno install, bun install, starship install | upstream | 2026-09-26/27 (target-tools A) | env-bound (os-release / chmod .exe); quoting machinery passed | none rubash-caused | `target-tools/results/A-*` |
| configure-class giants: bash53 (24,753 L), git (9,505 L), ltmain.sh (11,524 L), cmake bootstrap (2,124 L), php-8.3.16 (100,957 L) | upstream tarballs | 2026-09-27 (ecosys4 + target-tools B) | -n TIMEOUT family (O(N²)); real diffs #175/#177 | #175 #177 (+ #155/#130/#176/#178 perf) | `target-ecosys4/` |
| Homebrew: install.sh (--help parity), bin/brew --version, utils/os.sh | upstream | 2026-09-26/27 (ecosys3/6) | install.sh parity; bin/brew dies at #166 | #166 (family) | `target-ecosys{3,6}/` |
| MSYS2 / Git-for-Windows `/etc/profile` chain + `/etc/profile.d` (incl. real `lang.sh`) | local Git-for-Windows + MSYS2 | 2026-09-27 (ecosys6; #163) | 95% chain-identical; LANG export loss | #163 | `D:/repo/rubash/target-ecosys6/` |
| WSL `/etc/profile.d/*.sh` (8) + dpkg postinst (16) | local WSL | 2026-09-26/27 (target-tools D) | `-n` all identical; exec env-bound only | none | `target-tools/results/D` |
| makepkg (PKGBUILD + CLI/parseopts) | pacman upstream | 2026-09-27 (ecosys6) | PKGBUILD body byte-identical; CLI dead (#165+#166) | #165 #166 | `target-ecosys6/` |
| shellspec self-test (1134 cases) | upstream | 2026-09-27 (ecosys6) | 0 start (leak + $0 cascade) | #167 #166 | `target-ecosys6/` |
| bats-core self-test (test/bats.bats, 1910 lines) | upstream | 2026-09-27 (ecosys6) | 0 start (exec env silent skip) | #172 #166 | `target-ecosys6/` |
| portage ebuild.sh | gentoo upstream | 2026-09-27 (ecosys6) | dies at version gate | #174 | `target-ecosys6/` |
| as_fn_mkdir_p amplifiers / libtool `as_bourne_compatible` shapes | derived | 2026-09-27 (ecosys4) | perf family | #176 #178 | `target-ecosys4/work/` |
| bashdb (external debugger) | local fixture | ongoing | command coverage backlog | #28 + others | `target/bashdb-clean/` (present) |

**License inventory only, never executed (ecosys1 §A):** ble.sh (BSD-3) and
bash-preexec (MIT) were inventoried for bundling but explicitly not run in the
2026-09-26 wave — both were therefore **absent from coverage until the corpus2 lane
below** ran them.

## 2. corpus2 lane (this lane, 2026-09-27, rubash `9bf2df9e`, GNU 5.3.0 WSL baseline)

Method: identical LF corpus files run on both shells as **script files** (never
`wsl bash -c`); GNU side executed inside WSL writing artifacts to `/mnt/d/...`
directly (the `wsl.exe` stdout relay drops bytes — verified live again this lane);
`-n` parse + bounded (60 s) exec smokes; per-run rc/stdout/stderr captured.

Sandbox: `target/issue-suites/results/corpus2/` (corpora + drivers + probe bisects +
per-corpus `results/<name>/`). Pinned fixture copies: `tests/fixtures/corpus2/`.

| Corpus | Source / version | Verdict | Issues |
| --- | --- | --- | --- |
| bash-preexec.sh (566 L) | rcaloras/bash-preexec @d866ee | **divergence (exec)**: `PROMPT_COMMAND+=(...)` onto unset var stores serialized literal as scalar (`\x10` carrier leaks into value) → hook value corrupted; `declare -F` drops `-t`; `declare -f -t NAME` prints body. `-n` clean. | **#214, #217** |
| tldr (tldr-sh-client, 595 L) | raylee/tldr-sh-client @7d57134 | **divergence (exec)**: `usage > /dev/stderr` (function call redirect) — output vanishes / `Permission denied` on heredoc body. `-n` clean. GNU-side `unzip` absence is env-bound (panic path), not counted. | **#216** |
| hub.bash_completion.sh (387 L) | github/hub @410e841 | **clean** — `-n` identical; exec smoke with stubbed `_git`/`__git_list_all_commands`: function table and wrapped-listing output byte-identical (exercises the `eval "$(declare -f ... | sed ...)"` duplication chain). | none |
| ble.sh (29,601 L) | ble.sh nightly 2026-09-27, sha256 4b76374a… | **divergence (parse, P0-class)**: whole file unparseable (`-n` rc=2) and unsourcable; root = `$'…\'…`…'` lexer death at ble.sh:7319. Source-mode error misattribution to line 28 is #201/#204-family display, noted not re-filed. | **#215** |

Corpora considered and **rejected as already covered** (verified against §1 before
running; nothing was executed for them this lane): pyenv (ecosys5), conda.sh (ecosys5),
fzf ×2 (target-tools C, clean), starship (target-tools C), bash-it loader (ecosys1–3),
Git-for-Windows `/etc/profile.d/*` + `/etc/profile` (ecosys6/#163), MSYS2 profile
(ecosys6), GNU 83 `.tests` (ledger — all 83 files are the 83-suite; there are no
non-ledgered `.tests` files), Homebrew `Library/Homebrew/brew.sh` itself (family
covered: install.sh parity, bin/brew, utils/os.sh; brew.sh needs a full prefix+ruby —
env-bound, marginal), nvm/oh-my-bash/bash-completion/git-completion/rustup/ruby-build/
bashdb/dstack/new-exp/as_fn shapes (mission's explicit already-covered list).

## 3. Never run, with reasons (do not silently pick these up as "new")

| Candidate | Status | Reason |
| --- | --- | --- |
| kubectl bash completion | skipped | completion is generated at runtime by cobra; no static upstream copy (GitHub code search over kubernetes/kubernetes: 0 hits), no kubectl binary locally or in WSL. Same skip was recorded by ecosys3. |
| volta | unrunnable | Rust-native shim, no bash component (ecosys5). |
| miniconda installer | skipped | 140 MB download + real-install risk; conda.sh stub already covers activate/deactivate semantics (ecosys5). |
| ble.sh exec-mode (interactive) | parse-only this lane | raw-mode/`bind -x` readline replacement; blocked by #215 parse anyway. |
| actions/runner, checkout v1 entrypoint | corpus absent upstream | main .sh entrypoints removed from the repos (target-tools E lane). |
| hub alias script | not a static corpus | `hub alias -s` output is runtime-generated; the static completion file was run (clean). |

## 4. Reproducer fixture map (tests/fixtures/corpus2/repro/)

| Fixture | Issue | One-line shape |
| --- | --- | --- |
| `ansi_c_escaped_quote_backtick.sh` | #215 | `x=$'a\'b\`c'` → rubash rc=2 hunting `)`, GNU rc=0 |
| `append_array_literal_unset.sh` | #214 | `unset v; v+=("hello world"); declare -p v` → scalar serialized literal vs array |
| `declare_trace_attr.sh` | #217 | `declare -ft f; declare -F` drops `-t`; `declare -f -t h` prints body |
| `devstderr_function_echo.sh` | #216 | `u(){ echo hi; }; u > /dev/stderr` → output vanishes |
| `devstderr_function_heredoc.sh` | #216 | heredoc inside `u` + `u > /dev/stderr` → `Permission denied` rc=1 |

ble.sh itself is not pinned (29,601 lines / ~700 KB): fetch the nightly tarball pinned
in `corpora/SOURCES.txt`, or use the `ansi_c_escaped_quote_backtick.sh` fixture which
carries the full failure semantics.
> NOTE (wt4/ecosuite, 2026-09-27): this file is owned by the wt4/corpus2 lane.
> The ecosuite lane appends its dedicated mixed-suite coverage ONLY in the
> clearly-marked section at the end of this file and does not touch entries
> above. If corpus2 has not written its header/entries yet, they will be
> merged by the captain; treat everything between this note and the ecosuite
> divider as corpus2's territory.

---

# ===== SECTION: ecosuite lane (wt4/ecosuite, 2026-09-27) — APPENDED, DO NOT REORDER =====

Dedicated MIXED evaluation suites run by lane wt4/ecosuite (worktree base
9bf2df9e, rubash built from that commit). Oracle: WSL GNU Bash 5.3.0 script
file runs. Artifacts: `target/issue-suites/results/ecosuite/`; pinned
fixtures + per-case manifest: `tests/fixtures/ecosuite/`.

## 1. modernish use test suite — PAST CORE INIT (rubash#282 fixed); blocked at sys/cmd/harden parse

- Source: https://github.com/modernish/modernish @ 63bdae02 (0.17.23-dev),
  run as `bin/modernish --test -q -e`.
- GNU baseline: 389 tests — 384 succeeded, 5 skipped, 0 warnings, 0 xfail,
  0 unexpected failures. (gnu.out/gnu.err)
- State (2026-09-28, wt12/modinit lane, WSL GNU Bash 5.3.0 script-file oracle):
  plain `bin/modernish` init and `--use=_IN/sig` complete silently at GNU
  parity (rc=0, empty stderr); the fatal.sh battery passes; `use safe`
  works. The historical blockers are all closed: #218, #219 (long fixed),
  and rubash#282 (the fatal.sh battery / module-load abort this lane fixed:
  comment apostrophes corrupted quote state in the
  `stdin_source_has_unclosed_function_body` scanners, the misjudged group
  ran without the `__RUBASH_ALIAS_STREAMED` marker, and the re-spliced
  `$( ... )` body applied `alias let='let --'` twice — minimal reproducer
  and probe outputs under `target/issue-suites/results/issue282/`).
- Remaining blockers for the 389-test harvest (`--test -q -e`):
  1. env-bound — goodsh.sh requires `$PPID` continuity across an exec'd
     candidate shell; on Windows every native child reports PPID=1, so no
     candidate can ever match `$$` and modernish aborts with "Can't find any
     suitable POSIX-compliant shell!". This cannot pass on Windows with ANY
     engine (probed: `$(exec /d/Git/usr/bin/sh.exe -c 'echo $PPID')` -> 1).
     Harvest workaround (used by the LF fixture clone at
     `target/modernish-lf`): accept `"$$"|1` in goodsh.sh's two case
     patterns — inert for GNU (PPID==$$ there), 389/384 stays green.
  2. rubash-caused — `sys/cmd/harden.mm: line 404: syntax error near
     unexpected token '}'` (`rubash -n` rejects, GNU accepts): a nested
     eval-string parse divergence, next layer after #282. Kills `use sys`
     and therefore `--test` startup.
  3. rubash-caused — `use var` dies at `var/mapr.mm: line 380: File name too
     long` / "sys/cmd/mapr: failed to get ARG_MAX" (with stray
     PROCSUBST/PROCREDIR output) — Windows path-length/ARG_MAX layer.
- CRLF warning: the shared fixture `D:/repo/rubash/target/modernish-fixture`
  has CRLF line endings (GNU also rejects it — not a rubash signal); the LF
  clone `target/modernish-lf` (git -c core.autocrlf=false) is the runnable
  one.
- Verdict: core init + fatal.sh battery green at GNU parity; the 389-test
  harvest is blocked by the two rubash-caused layers above (harden.mm parse,
  mapr ARG_MAX). Older probe chains under
  `target/issue-suites/results/ecosuite/modernish/probes/` (p9, p37 and
  p11a..p45 bisect artifacts) remain valid history.

## 2. mvdan/sh (shfmt) parser corpus — RUN COMPLETE: 559 snippets, 16 divergences

- Source: syntax/filetests_test.go `fileTest(...)` input strings extracted
  from https://github.com/mvdan/sh @ aebdf2b8 (HEAD has the corpus inline in
  Go; older tags no longer carry .txt filetests — extraction script kept at
  `target/ecosuite/extract_mvdan.py`).
- Method: `rubash -n` vs `wsl bash -n` per snippet, exit-code parity
  (matrix.txt / diverge.txt / per-file stderr under results/ecosuite/mvdan-n/).
- 543/559 parity. Divergences (all GNU-as-oracle):
  - rubash#220 (8): accepts mksh/zsh-only operators GNU rejects — `&|`,
    `&>|`, `&>>|`, `>>|` chains, case `;|` fallthrough, `<->`, `<5-10>`.
  - rubash#221 (4): accepts GNU-invalid input — `( )` empty subshell,
    `while false; do; done`, `foo=([)`, `[[ a == (b|c)* ]]` without extglob.
  - rubash#222 (5): rejects GNU-valid input — `((# 1 + 2))`, `{ foo } }; }`,
    `${foo/$a/$'\''}`, heredoc after `cat <<EOF ;;` in a case arm,
    `[[ a =~ ( ]]<>;&) ]]`.
- All 16 snippets pinned under `tests/fixtures/ecosuite/mvdan-n/` for CI.
- Verdict: parse-conformance 97.1% on this corpus; both over-lax and
  over-strict families live.

## 3. nvm own test suite — RUN (bounded 12-test slice): 6 pass / 6 fail

- Source: https://github.com/nvm-sh/nvm @ a885b885 (v0.40.8). NOTE: current
  nvm master no longer uses bats; test/fast entries are plain executable
  /bin/sh scripts run per-file (the mission's "bats suite" premise is
  outdated for nvm HEAD — documented here).
- Method: first 12 test/fast plain files, each under `wsl bash` (index-based
  WSL-side runner; wsl.exe passthrough corrupts spacey/`$` filenames) and
  under rubash from the same cwd, 60s per test. Classification cross-checked
  against Git Bash real bash 5.3.15 on the same files.
- GNU: 12/12 pass. rubash: 6 pass / 6 fail.
  - 4 fails environment-bound (Windows platform detection in nvm.sh
    v0.40.8: `NVM_NODE_BINARY=node.exe` vs mock's plain `node` in
    `nvm_is_version_installed`; identical failure under Git Bash bash).
    Not counted against rubash. Affects: deactivate, install
    --reinstall-packages-from, uninstall clean-up-aliases, uninstall
    remove-directory.
  - 2 fails rubash-caused — rubash#223: "nvm exec/run warn fallback" and
    "nvm uninstall inferred version" leak `/c/Program Files/nodejs/node.exe`
    and fail their assertions under rubash while Git Bash passes the same
    files. Root cause not isolated in this lane (no src/ fixes allowed).
- Verdict: nvm suite usable as a rubash smoke suite on Windows at a ~50%
  pass rate after subtracting platform-bound noise; #223 is the actionable
  rubash bug.

## 4. bats-core self-suite — ENTRY CHAIN RUNS, TEST EXECUTION BLOCKED

- Source: https://github.com/bats-core/bats-core @ 52439ebf.
- `rubash bin/bats --version` works (Bats 1.14.0) after two workarounds;
  baseline GNU runs green on cat-formatter/tagging/filter slices (3 files,
  21 tests) both pristine and on the patched copy.
- Blockers:
  1. env-bound (winuxsh env): `export -f` + `exec env BATS_ROOT=... bats`
     (bin/bats:89-90) loses the exported function at the external `env`
     boundary -> `bats_readlinkf: command not found` (libexec bats:119).
     rubash exports `BASH_FUNC_f%%=...` correctly and direct rubash
     children import it; the loss is in the env utility. Harvest workaround:
     patched copy `target/ecosuite/bats-harvest` (patch: drop `env` from the
     exec, identity `bats_readlinkf` normalizing `D:/` to `/d/`; GNU green
     on the same patch).
  2. rubash-caused — rubash#224: `$0`/`BASH_SOURCE` of scripts invoked by
     absolute /d/-style path is rewritten to `D:/` form while `$PWD` stays
     `/d/`; downstream `${BATS_TEST_FILENAME##*/}` fails on backslash paths
     producing the invalid redirect target
     `.../1-D:\repo\...\tagging.bats.src` -> every test file dies
     `not ok 1 bats-gather-tests` (rc=1). Instrumented argv logs saved
     (exec-suite receives /d/ form; gather-tests receives D:\ form; the
     isolated repros of the intermediate steps are clean — exact conversion
     point not pinned; see issue for the matrix).
- Verdict: self-suite not harvestable until #224 lands; the runner chain up
  to gather-tests is otherwise functional under rubash.

## 5. Fallbacks — SKIPPED as already covered

- mksh check.t: covered by earlier rounds (rubash#23/#24 note 153+ and 436+
  DIFF ledgers).
- busybox ash tests: covered by wt-busybox workspace (rubash#32..#57).
- Both skipped per the "already covered" rule; no reruns in this lane.

## Issue ledger from this lane

| issue | suite | class |
| --- | --- | --- |
| rubash#218 | modernish | rubash-caused (assignment-RHS `\\` collapse) |
| rubash#219 | modernish | rubash-caused (parser: brace group + case + trailing comment) |
| rubash#220 | mvdan/sh | rubash-caused (over-lax: mksh/zsh operators) |
| rubash#221 | mvdan/sh | rubash-caused (over-lax: GNU-invalid constructs) |
| rubash#222 | mvdan/sh | rubash-caused (over-strict: GNU-valid constructs) |
| rubash#223 | nvm | rubash-caused (nvm exec/run/uninstall system-node leak) |
| rubash#224 | bats-core | rubash-caused ($0/BASH_SOURCE D:/ vs $PWD /d/ path domains) |

Environment-bound findings recorded (no rubash issue): modernish goodsh
PPID-across-exec impossibility on Windows; bats bin/bats `exec env` exported
function loss at the external env utility; nvm v0.40.8 `_win` node.exe
mock mismatch (fails under Git Bash too).
Context and full evidence: `docs/LINUX-RUN-STATUS.md`. Environment: both the
shell-under-test (rubash Linux ELF, `RUB_OVERRIDE` +
`RUBSIDE_PATH=$BASE:/usr/local/bin:/usr/bin:/bin`) and the baseline (GNU Bash
5.3.0) ran inside the same WSL instance via `scripts/true-baseline.sh` —
pure engine diffs, `env=0` on every suite.

Coverage added by this lane:

- **GNU suites (bounded slice, 24 suites)**: 17 byte-identical (arith,
  arith-for, more-exp, exp, comsub, comsub2, dstack, dstack2, jobs, heredoc,
  case, braces, cond, globstar, procsub, printf, set-e); trap and redir end
  in hangs (rubash#225/#227, unkillable per #226); glob 40 / extglob 16
  ordering (#235, plus #236); new-exp 6 (#239); read 2 (#238); ifs-posix
  timeout (#237). Artifacts:
  `target/issue-suites/results/true-baseline/<suite>/{gnu.out,rb.out,*.rc}`.
- **Real-world script shape**: nvm.sh v0.40.3 (4661 lines) — `-n` parse
  clean, `. nvm.sh --no-use` loads with byte-identical stderr, `nvm_version`
  works. This is the first full-size real script verified end-to-shape on
  the Linux target.
- **Smoke matrix** (12 probe classes, `target/wslrun/`): all core classes
  byte-identical after the lane's chmod/test -x/uname fixes; residual gaps
  are the filed issues #228-#234.

Not yet covered on Linux (next lanes): the remaining 59 GNU suites (notably
coproc, vredir, lastpipe, dbg-support, history, complete, tilde, posixexp
family), interactive/readline behavior (no tty exercised), locale variants
beyond en_US.UTF-8, and the `--version`/banner policy decision (#240).
## Perf-suite lane section (wt4/perfsuite; baselines in docs/PERF-BASELINE.md)

Per-probe corpus entries for the performance suite. Every probe is a
committed file under `benchmarks/`; ratio/status from the 2026-09-27 baseline
(rubash 9bf2df9e debug vs WSL GNU bash 5.3.0). Ratios >= 10x are tracked in
rubash#241 (parse family) and rubash#242 (hot path); the #206-shape hang is
rubash#243.

| Probe | Corpus entry | Shape family | Baseline ratio |
|---|---|---|---|
| 01-startup-empty.sh | empty script | startup floor (#158) | 13.1x |
| 02-startup-fndef.sh | 1 fn def + call | startup lazy-init (#158) | 12.7x |
| 04-loop-true-builtin-x2000.sh | builtin loop | hot path (#157/#186) | 52.4x |
| 05-arith-x5000.sh | `(( ))` loop | arithmetic (#156) | 49.8x |
| 06-strconcat-x5000.sh | `s+=x` loop | assignment (#157) | 76.6x |
| 07-fncall-noop-x5000.sh | fn call loop | dispatch (#157/#186) | 94.1x |
| 08-cmdsub-true-x1000.sh | `$(true)` loop | subshell spawn | 1.2x |
| 09-external-uname-x300.sh | external spawn loop | fork/exec (env-bound) | 1.5x |
| 10-pathmiss-x100.sh | PATH miss loop | lookup storm (#159) | 10.9x |
| 11-pipeline-yes-head.sh | `yes \| head -100000` | pipeline streaming (#157/#206) | 27.6x |
| 12-pipe-echo-read-x2000.sh | `echo \| while read` x2000 | builtin pipeline (#157) | 5.7x |
| 13-readloop-gen-x2000.sh | gen \| while read | read-loop throughput | 106.2x |
| 14-glob-srcrels-x100.sh | `src/*/*.rs` glob loop | glob (#157; env-bound: GNU drvfs) | 0.1x* |
| 15-expansion-x5000.sh | param-expansion mix | expansion hot path | 76.1x |
| 16-parse-flat8000.sh | 8000 assignments | flat parse throughput (#155) | 123.3x |
| 17-parse-flat8000-n.sh | same, `-n` only | parse-only throughput | 39.2x |
| 18-nested-brace-nst1-d200.sh | one-line nesting D=200 | #176 nst1 | 96.5x |
| 19-nested-brace-nst2-d200.sh | two-line nesting D=200 | #176 nst2 (rubash parse-FAIL canary) | RC2 |
| 20-as-fn-mkdir-p-rep40.sh | autoconf unit x40 | #178 rep-40 | 65.1x |
| 21-configure-head1374-n.sh | GNU bash configure head 1374 L, `-n` | #130/#155/#178 family | 356.8x |
| 22-configure-full-n.sh | GNU bash configure full 24753 L, `-n` | #130/#155/#178 family | TIMEOUT |
| 23-nvm-parse-n.sh | nvm.sh v0.40.8, `-n` | #130/#155 family | 852.2x |
| 24-nvm-load.sh | nvm.sh v0.40.8 sourced | #130/#155 family (load) | 379.0x |
| 25-yes-head-read.sh | `yes \| head \| while read` | #206 hang canary | TIMEOUT |
| corpus/nvm.sh | nvm-sh/nvm v0.40.8 (a885b885, MIT) | vendored verbatim, offline | — |

*14's ratio measures WSL drvfs vs NTFS, not the shells (GNU side inflated);
tracked for rubash-side regressions only. Probe 03 (`-i -c exit`) was retired:
GNU bash hangs on interactive invocations without a tty in WSL.


# ===== SECTION: source audit lane (wt5/audit, 2026-09-28) — APPENDED, DO NOT REORDER =====

This lane ran a line-level GNU C source vs Rust owner gap audit (read-and-probe,
no src/ changes) over six areas: builtins option tables, redir.c redirection
forms, subst.c parameter operators, parse.y grammar productions, jobs/trap
display, variables.c attribute semantics. 336 behaviors probed against WSL GNU
Bash 5.3.0 script-file probes; 14 divergences + 1 missing family verified and
filed (#261, #263-#272, plus a %5ld evidence comment on #233).

Full per-area tables, C anchors, classifications and probe verdicts:
**docs/SOURCE-AUDIT.md** (single source of truth for this lane).
Probe harness: target/issue-suites/results/source-audit/cmp.sh; raw
artifacts under target/issue-suites/results/source-audit/{probes,out}/;
durable reproducers under tests/fixtures/audit/.

# ===== SECTION: corpus3 lane (wt5/corpus3, 2026-09-27) — APPENDED, DO NOT REORDER =====

Corpus-discovery lane run by wt5/corpus3 (worktree base `86818357`, rubash built
from that commit; oracle WSL GNU Bash 5.3.0 script-file runs, configure-class
with `PATH=/usr/local/bin:/usr/bin:/bin`). Sandbox:
`target/issue-suites/results/corpus3/` (corpora + SOURCES.txt + work/ probes +
results/<case>/{rub,gnu}.{out,err,rc,meta}). Pinned repro fixtures:
`tests/fixtures/corpus3/repro/`. All four mission candidates were verified
against §1/§3 as never-run before executing (git's *configure* ran in ecosys4;
git's *test harness* had not; FFmpeg/OpenSSH configure absent from §1; bash-it
coverage in ecosys1-3 counted loader/component files only, no test-suite run).

## Verdict table

| Corpus | Source / version | Verdict | Issues |
| --- | --- | --- | --- |
| git test harness (`t/test-lib.sh` framework) | git v2.47.2 tarball (t/ extracted, 1136 files) | **partially runnable**: `-n` parity CLEAN on all 5 harness files; t0000-basic `--run=1,4` runs end-to-end under rubash (2/2 selected pass, full plan line `# passed all 92 test(s)`); BUT `test_oid_init` loads only 1 row per file (rubash#260 read-loop stdin bug) → 4 `undefined key` BUG lines at every startup; nested sub-test machinery blocked by env: `uname -s`=MINGW64 → harness wraps `pwd` as `builtin pwd -W` (MSYS-only extension, GNU bash has no `-W`; rubash is GNU-faithful) → `$(pwd)` empty in test bodies; `/d/`-style absolute paths given to native git.exe children are reinterpreted as drive-relative (`git init /d/...` created `D:\d\...`) — MSYS arg-conversion gap, ecosys4 precedent (env-bound) | **#260** (rubash-caused) |
| FFmpeg configure | tag n7.1.1 raw file, 8345 L, sha256 e7c000ab… | **clean**: `-n` rc=0/0 parity (rubash 14 s vs GNU <1 s = known #155/#130 perf family); `--help` stdout byte-identical 27797 B rc=0/0; invalid-flag stdout byte-identical (incl. `$0` rendering) rc=1/1; only stderr noise = missing-source-tree sed diagnostics rendered by different sed binaries (env-bound) | none |
| OpenSSH configure | openssh-9.9p2 tarball, 27712 L, sha256 91aadb60… | **exec clean, parse timeout**: `-n` rubash TIMEOUT@110 s (rc=124) vs GNU 0 s rc=0 — known #155/#130 O(N²) family (ecosys4 already has configure-scale evidence, not re-filed); `--help` (after touching stub `ssh.c`) stdout+stderr **byte-identical** 8059 B rc=0/0; invalid flag byte-identical both streams rc=1/1 | none new (perf: #130/#155) |
| bash-it own test suite (`test/run` + 25 `.bats`) | master @4725d29d; bats submodules pinned per .gitmodules (bats-core 6636e2c2 = v1.9.0; also probed 52439ebf) | **blocked both sides, no rubash divergence measurable**: rubash runner-chain (`test/run`: git submodule init/update, git diff gate, exec bats) works up to the KNOWN env-bound `exec env` exported-function loss (`bats_readlinkf: command not found`, ecosuite §4 blocker 1 — not re-filed); oracle side itself executes **0 of 13** tests in lib/log.bats silently under GNU 5.3.0 with BOTH bats 1.9.0 and 1.14.0 (bats' own suite: 113/113 green on the same fixture → bash-it × bash-5.3 interaction, oracle-side env incompat); raw `.bats` `-n` symmetric rc=2 (bats macros, no signal) | none (ecosuite blockers) |

## New issues from this lane

| issue | corpus | class |
| --- | --- | --- |
| rubash#260 | git test-lib (and general idiom) | rubash-caused: external command inside `while read` loop with file redirect consumes the loop stdin → 1 iteration (GNU 5); breaks `test_oid_cache` (`t/test-lib-functions.sh:1725`) → every git t-file startup |
| rubash#262 | OpenSSH configure lane (mistyped path exposed it) | rubash-caused: script-file open failure exits 1 instead of 127 (ENOENT) / 126 (EISDIR); C owner `shell.c:1572 open_shell_script()` `sh_exit((e==ENOENT)?EX_NOTFOUND:EX_NOINPUT)`, `shell.h:65-66` |

## Environment-bound findings recorded (no issue, per ecosys4 MSYS-arg precedent)

1. **rubash `uname -s` = `MINGW64_NT-10.0-19044` claims MSYS identity but MSYS
   runtime extensions are absent** (correct per GNU source): git's test-lib
   MINGW branch wraps `pwd () { builtin pwd -W }` (MSYS-bash extension; GNU
   bash 5.3 pwd has only -L/-P) → every `$(pwd)` inside harness test bodies is
   empty → nested sub-test `cd`/paths collapse. Also sets NATIVE_CRLF /
   WINDOWS prereqs and `GIT_TEST_CMP=git diff…`. Policy decision (not a
   GNU-compat bug): either implement MSYS extensions behind the MINGW uname
   identity or change the identity string.
2. **`/d/`-style absolute paths handed to native children**: `git init
   "/d/…"` under rubash created `D:\d\…` (drive-relative reinterpretation by
   the native child); MSYS2 bash converts args for native children, rubash
   does not. Same class as ecosys4's `cmd //c` note. Workaround used:
   `TEST_OUTPUT_DIRECTORY=.` (relative trash dir) — which in turn trips the
   harness's absolute-TRASH assumption ("Tests passed but trash directory
   already removed" rc=1), i.e. the harness needs either fix above or a real
   built tree to be harvestable.
3. **bats 1.9.0/1.14.0 × bash-it master × bash 5.3**: 0 tests executed on the
   GNU oracle itself (bats self-suite green) — re-try bash-it suite only
   after bash-it or bats adapts to 5.3.

## Positive parity results worth keeping

- Autoconf-generated `--help`/`--version`/error-path exec parity now holds for
  bash53, git, php, ltmain (ecosys4), **FFmpeg** and **OpenSSH** (this lane):
  the autoconf + hand-written-torturous-configure exec paths are byte-clean
  under rubash; the only configure-class residue is the `-n` O(N²) family.
- git t/ harness *startup* (option parsing, chainlint via perl, prereq setup,
  test selection, TAP emission, fd 3/4 juggling, `--run`/`--root` handling,
  shebang `#!D:/…exe` re-exec of nested test scripts) is byte-parity against
  GNU up to the two blockers above.

## Reproducer fixture map (tests/fixtures/corpus3/repro/)

| Fixture | Issue | One-line shape |
| --- | --- | --- |
| `read_loop_child_consumes_stdin.sh` | #260 | `while read l; do expr …; done < f` → rubash 1 iteration, GNU 5; `</dev/null` on child restores 5 |
| `script_open_exit_status.sh` | #262 | `bash -n missing.sh` → GNU rc=127 / rubash rc=1 (dir-as-script: 126 vs 1) |

# ===== SECTION: ecosweep lane (wt10/ecosweep, 2026-09-28) — APPENDED, DO NOT REORDER =====

Fine-grained per-item pass over the bash plugin ecosystem. Worktree base `e207b0df`
(rubash release build from it); oracle: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`),
script-file probes only. Sandbox: `target/issue-suites/results/ecosweep/` (harness +
per-item artifacts + probes/), corpora cloned LF-normalized under
`target/ecosweep-corpus/`.

Dedup check against §1–§4 BEFORE running: oh-my-bash (repo-level + 5-theme matrix +
full init, ecosys1-3), bash-it, bash-completion, nvm, rustup, modernish, mvdan, ble.sh,
bats — all coarse layers already covered, not re-run. **bash-preexec** (corpus2, #214/#217)
and **fzf** (target-tools C, clean) and **starship** (target-tools C, #202) were likewise
NOT re-run wholesale — bash-preexec only got a 2-probe known-issue status check (below).
The new layers this lane added: (a) OMB *per-theme/per-plugin/per-alias/per-completion*
matrices (never done item-by-item), (b) first contact with liquidprompt, bash-sensible,
complete-alias, mathiasbynens dotfiles, thoughtbot dotfiles, direnv *bash hook template*
(stdlib.sh was covered; the hook is a distinct artifact).

## Method

Carrier: `target/release/rubash.exe --rcfile <probe-rc> -i </dev/null`, rc sets
`DISABLE_AUTO_UPDATE=true`, `OSH`, the OMB config arrays, `cd` into the worktree git
repo, `source $OSH/oh-my-bash.sh`; assertions via markers: a) `@@SOURCE-DONE`,
b) PROMPT_COMMAND loop `for pc in "${PROMPT_COMMAND[@]}"; do eval "$pc"; done`
(stderr to per-item .pcerr), c/d) PS1 dumped raw between `@@PS1-START/END` and scanned
post-hoc (python) for escape structure / carrier-PUA leakage. Every run under
`timeout 20`. Harness: `target/issue-suites/results/ecosweep/omb/{run-item,drive-kind}.sh`
+ `post.py`; verdicts in `{themes,plugins,aliases,completions}.verdict.txt`.

Pitfalls hit (recorded for future lanes):
- The local `C:/Users/Administrator/.oh-my-bash` install is a **CRLF** checkout (GNU
  cannot read it at all). GNU-side theme comparisons therefore use an LF clone at
  `target/ecosweep-corpus/oh-my-bash-lf` (same commit abf8461). All corpus clones were
  LF-renormalized the same way (`git config core.autocrlf false && git reset --hard`).
- `~/.local/bin/env` (winuxsh env) silently swallows child stdout when used as a wrapper
  — use plain shell assignments instead.
- bash-completion master's `_comp_abspath` only recognizes `/*`-shaped absolutes;
  feeding rubash `D:/…` sources degrades the base (missing `startup-core/` pickup).
  POSIX-form `/d/…` paths on the rubash side are required for symmetric comparisons
  (env-bound, upstream assumption — not a rubash issue).

## A. oh-my-bash item-by-item (OSH abf8461 = local install = LF clone)

| Kind | Items | Result | Reds |
| --- | --- | --- | --- |
| themes | 83 | 69 fully clean (a/b/c/d OK); 12 PS1-PUA hits are theme-authored powerline/nerd glyphs in source (agnoster, nekolight, powerbash10k, powerline* ×6, absimple) or deliberate `$'\1…\2'` title markers / printf-BEL (kitsune, morris) — by design, GNU-equivalent | binaryanomaly + rjorgenson: U+E000+U+E01C carrier pairs corrupt PS1 → **#296**; random: `${a[RANDOM%83]}` → **#299**; half-life: `_omb_util_split` positional-param IFS failure → **#298**; iterate: interactive COLUMNS/LINES never initialized → **#300**; every run's stderr carries post-exit REPL echo noise → **#297** |
| plugins | 36 | **36/36 green** (only "tool not found" notices from the plugins themselves) | none |
| aliases | 9 | **9/9 green** | none |
| completions | 59 (with bash-completion base sourced first) | **58/59 green** (base-load marker `@@BC-BASE-OK` 59/59; verdict table's svn row is a false green — markers still print after the syntax error) | svn.completion.sh: `shopt -s extglob` mid-source kills the next `function NAME()` header → **#302** |

Notable positive: the interactive OMB load that #251 described (13/326 functions) is gone —
all 83 themes now source the full lib chain, run PROMPT_COMMAND and build a PS1.

## B. previously-uncovered ecosystem sources

| Corpus | Source / version | Verdict | Issue |
| --- | --- | --- | --- |
| liquidprompt | nojhan/liquidprompt @afd7e83 (2026-09-13) | **loads and renders under rubash**: source rc=0, PROMPT_COMMAND runs, PS1 built with correct `\[`+ESC structures (byte-shape parity vs GNU; text differs only by user/host/path env), 252 `__lp_*`/`_lp_*` functions defined (GNU 244 — env-conditioned set, none load-blocking) | none new (#297 stderr noise only) |
| bash-sensible | mrzool/bash-sensible @eb82f9e (v0.2.2) | **clean**: all options/shopt/HIST*/trap state byte-identical; `bind` warnings line-for-line identical (path string differs only) | none |
| complete-alias | cykerway/complete-alias @7f2555c | **clean**: source + alias registration + `complete -F _complete_alias` + simulated COMP_WORDS dispatch — COMP_WORDS/LINE/POINT rewrite and final COMPREPLY byte-identical, both with and without a bash-completion base (needs POSIX-form paths on the rubash side, see pitfall above) | none |
| mathiasbynens/dotfiles | @b7c7894 (.bashrc → .bash_profile → .{path,bash_prompt,exports,aliases,functions}) | shopt chain + PS1 structure parity; alias table diff = env items ($SHELL, macOS guards) PLUS one real divergence: backslash dropped from single-quoted alias body | **#301** |
| thoughtbot/dotfiles | @939a270 | **no bash component** (zsh-only repo: zshenv/zshrc/zprofile) — recorded, nothing to run (same class as volta in §3) | none |
| direnv bash hook | direnv @master `internal/cmd/shell_bash.go` template, `{{.SelfPath}}` stubbed to true per side | **clean / byte-identical**: unset→array, scalar, and array PROMPT_COMMAND registration shapes, `";${PROMPT_COMMAND[*]:-};"` dedupe gate, `declare -p` array detection, `_direnv_hook` exit-status preservation | none (stdlib.sh already #202) |
| bash-preexec (status only) | corpus2 coverage; pinned fixtures | **#214 and #217 both fixed at e207b0df** — fixtures run byte-identical to GNU; status comments posted on both issues | none new |

## Issue ledger from this lane

| issue | found via | class |
| --- | --- | --- |
| rubash#296 | binaryanomaly/rjorgenson themes | assignment-RHS word with control-byte var adjacent to live `$( )` leaks U+E000+U+E0xx carrier into the value (arg position clean; double pass escalates to U+E400-escaped) |
| rubash#297 | harness stderr noise (all 83 theme runs) | `exit` in `--rcfile` init does not terminate the interactive shell — stdin lines execute afterwards (GNU exits); prompt echo + spurious `unexpected EOF looking for ')'` when PS1 has parens |
| rubash#298 | half-life theme / lib/utils.sh `_omb_util_split` | unquoted positional parameter in array assignment is not word-split on IFS (`set -- a.b.c; IFS=.; A=($1)` → 1 element, GNU 3) |
| rubash#299 | random theme | `RANDOM` (dynamic special var) expands empty in array-subscript arithmetic (`${a[RANDOM%3]}`) |
| rubash#300 | iterate theme | interactive startup never binds COLUMNS/LINES without a tty (GNU: 80/24 via readline defaults) |
| rubash#301 | mathiasbynens .aliases:148 | backslash inside a single-quoted alias body dropped from the stored alias value |
| rubash#302 | svn.completion.sh | `shopt -s extglob` executed inside a sourced file breaks parsing of subsequent `function NAME()` headers (GNU accepts) |

## Artifacts

- Matrices: `target/issue-suites/results/ecosweep/omb/{themes,plugins,aliases,completions}.verdict.txt`
  (+ per-item `.out/.err/.rc/.pcerr` in the same-named dirs; raw verdict note: completions/svn
  row is a false green — see #302).
- Probes + GNU comparisons: `target/issue-suites/results/ecosweep/probes/` (p1–p15 issue
  isolation ladders, b01–b18 ecosystem probes, t1–t3 rcfile probes, diag* theme diagnostics).
- Corpora (LF): `target/ecosweep-corpus/{oh-my-bash-lf,liquidprompt,bash-sensible,complete-alias,dotfiles-mathiasbynens,dotfiles-thoughtbot,bash-completion,direnv}`.

# ===== SECTION: ecosweep2 lane (wt13/ecosweep2, 2026-09-28) — APPENDED, DO NOT REORDER =====

Fine-grained SECOND-PASS over already-covered corpora + new-corpus mining.
Worktree base 116b12aa (release build from it); oracle WSL GNU Bash 5.3.0
(/usr/local/bin/bash), script-file probes only. Sandbox:
target/issue-suites/results/ecosweep2/; corpora fetched LF via gh-api tarballs under
target/ecosweep2-corpus/ (pins in ecosweep2-corpus/SOURCES.txt; github.com web
fetch was down - api.github.com tarballs worked).

Dedup check BEFORE running: sections 1-3 + ecosweep section read in full. Not re-run:
GNU 83, git-completion, oh-my-bash item matrices, modernish, mvdan, bats,
bash-sensible/complete-alias/dotfiles/direnv-hook/starship/fzf (all ecosweep-covered).
Layer additions chosen per mission A/B lists only.

## Method (and pitfalls recorded for future lanes)

- Comparison = same script file on both shells; all output through files
  (>file 2>file), never terminal-captured - the wsl.exe stdout relay drops/garbles
  bytes intermittently and produced misleading interleavings twice this lane.
- while-read loops that invoke wsl inside MUST guard stdin (</dev/null on the wsl
  call) or wsl.exe consumes the loop's stdin (ate our matrix file on run 1).
- Path depth: harnesses under results/ecosweep2/<x>/ need ../../../../ to reach
  target/ecosweep2-corpus/ (bit us three times).
- GNU-side per-item interactive loops (bash -i --rcfile for bash-it) hang >25 s/item
  in this WSL instance (oracle-side infra; not counted as any shell's bug) - the
  bash-it GNU comparison was sampled (15 items, non-interactive) instead.
- rubash pwd inside command substitution renders D:/-style paths; harmless for
  sourcing but shows up in diffs (path-domain normalization applied where mattered).

## A. second-pass results

| # | Corpus (prior coverage) | New layer this lane | Verdict | Issues |
| --- | --- | --- | --- | --- |
| A1 | bash-completion 2.18 (ecosys1/3: load + 11 driven) | driven interaction matrix: 72 queries over 58 core completions (COMP_WORDS/CWORD/LINE/POINT + complete -p to -F dispatch per ecosys3 probe-bcomp method; harness bc/) | 39/72 byte-parity; 31 DIFF all environment-bound (target binary absent/version drift: curl rsync free etc, awk dialect, watch OSTYPE linux/darwin gate, ip/man read real binaries+config). Engine reds found during the run: false-nounset on the ${var#pat} family kills the whole -n : class under set -u; .gitignore gets sourced at base load (GLOBIGNORE-empty dotglob) | #311 (+widen comment), #313 |
| A2 | bash-it (ecosys1-3: full load only, #161) | per-component: 50 aliases + 90 completions + 81 plugins enabled one at a time and loaded via real bash_it.sh (interactive rcfile, timeout 20 each) | rubash 221/221 rc=0 loads (ecosys-era full-load break is history). GNU sampled 15 (interactive GNU loop unusable, see pitfalls): 12/15 differ by exactly ONE auto-generated function (_bash-it-component-completion-callback-on-init-aliases, FUNCS 161 vs 162); apt item = env (Git Bash reproduces); 2 identical | #316 |
| A3 | liquidprompt (ecosweep: load+render only) | its own shunit2 suite (29 test_*.sh files, shunit2 v2.1.8 vendored into tests/) | GNU 28/29 (test_utils fails on drvfs-cwd path-shortening - oracle-side env). rubash 20/29: 3 files die to #311-family nounset (liquidprompt:1736 ${display_path//[!\/]} etc), 2 files unparseable (nested multi-line funcdef in loop-in-function), test_array 2/3 asserts (guard family); test_git/test_terminal_device env (symlinks / no tty) | #314, #315, #311-family |
| A4 | ble.sh (corpus2: -n parity only, #215 closed) | full source load (--noattach + bleopt_connect_tty TTY-gate bypass; real-PTY GNU run via script -qec) | GNU under real PTY: rc=0, 1902 functions. rubash without PTY passes the TTY gate (33 fns) then aborts at ble.sh:598 noediting gate - interactive startup keeps emacs OFF and $- lacks H without a tty; -i -c $- diverges the other way (hBc vs himBH). PTY-less frameworks (ble.sh, readline replacements) blocked at this layer | #312 |
| A5 | nvm (ecosuite section 3: 12-file slice, #223 closed) | full fast-suite: all 36 plain test/fast files (cwd=test/fast - the tests source ../../nvm.sh; index-based WSL runner for spacey names) | GNU 36/36. rubash 21/36: 11 also fail under Git Bash (env: node.exe naming, symlinks, hashing, MANPATH); 4 fail under rubash only (uninstall-inferred, use-system, .nvmrc-system, nvm-exec) - trace shows nested-comsub stdout leaking into the outer capture (inner substitution returns empty); standalone reductions PASS, mechanism depends on nvm.sh context | #289 (evidence comment), #223 (status comment) |

## B. new corpora

| Corpus | Source / pin | Verdict | Issues |
| --- | --- | --- | --- |
| Homebrew/install install.sh | @04dfcac, 1238 L | 4/4 driven paths byte-parity (--help, -h, -q --help, unknown-flag) - re-verifies ecosys coverage at HEAD | none |
| Homebrew/install uninstall.sh | @04dfcac, 562 L (never run before) | aborts at platform gate "Unsupported system type MINGW64_NT-10.0-19044" + hard /usr/bin/sudo - by-design env (MSYS identity per #154 policy); GNU proceeds and prints usage | none (env) |
| Homebrew unattended.sh | - | does not exist in Homebrew/install (only install.sh/uninstall.sh + ruby tests); premise recorded as invalid | - |
| pyenv rehash/shell/exec paths | @699e27f (section 1 covered via 127-cascade; blockers #154/#171 now CLOSED) | pyenv --version / versions / rehash (shim created) / prefix / shell-integration-error at byte parity; rbenv rehash + versions parity. pyenv init - output diverges via shell-name detection (/proc/$PPID/cmdline gives "rubash.exe"): PYENV_SHELL=rubash.exe, per-shell completion file not sourced, degraded case pattern - faithful process-name detection, product-identity class | none (#154 class) |
| sdkman sdkman-init.sh chain | @66767df, init + 21 modules | init chain sources rc=0 both sides with byte-parity stdout; sdk function needs a fuller installed layout on BOTH sides (symmetric 127s); only stderr delta is winuxsh find path-prefix wording | none |
| docker/cli completion | @7fc2dff, contrib/completion/bash/docker 5598 L | -n parity; full source rc=0; complete -F _docker registration parity; 6 driven queries 0/0 parity - candidate generation needs the docker binary (Git-bundled docker exists Windows-side, none in WSL: asymmetric env, candidate layer not measured) | none |
| kubectl bash completion | - | skip re-confirmed: no kubectl in WSL or Windows (probed), no static upstream copy (section 3 precedent stands); synthesizing from cobra's template rejected as unfaithful | - |
| ohmyzsh tools/*.sh | @83a0ec7 (tools/ only; repo is zsh-first) | bash-compatible 4 files (install/uninstall/theme_chooser/require_tool) exec-abort parity 4/4 (rc + stdout byte-identical); changelog/check_for_upgrade/upgrade are zsh syntax, rejected by BOTH shells at -n (rc=2/2) | none |
| concourse task-script family | concourse/git-resource @d915957 - NB: repo restructured, opt/resource/ no longer exists; assets/*.sh (10 files incl. check_branches/tags, common) | 10/10 -n parity; exec drive of check scripts rc-parity with line-for-line identical stderr (jq: command not found on BOTH sides - symmetric env) | none |

## Issue ledger from this lane

| issue | found via | class |
| --- | --- | --- |
| rubash#311 | bash-completion _comp_initialize -n : / liquidprompt 3 suite files | rubash-caused: false nounset on expansions with patterns (${var#[[:space:]]}, ${var//[!\/]}, nested "${arr[i]}" - 8-shape matrix in issue comment) |
| rubash#312 | ble.sh noediting gate / interactive defaults matrix | rubash-caused: no-tty -i keeps emacs off, $- missing H (--rcfile/-s), -i -c $- = himBH vs GNU hBc |
| rubash#313 | bash-completion compat-dir scan sourcing .gitignore | rubash-caused: GLOBIGNORE set-but-empty wrongly enables implicit dotglob (pathexp.c:507 setup_glob_ignore two-branch semantics) |
| rubash#314 | liquidprompt test_disk/test_ram rc=2 | rubash-caused: multi-line funcdef in loop-in-function fails to parse (6-line repro) |
| rubash#315 | liquidprompt test_array 2/3 | rubash-caused: ${a[2]+x} false-SET inside ${a[@]+...}; empty quoted element dropped in unquoted slice assignment |
| rubash#316 | bash-it sample FUNCS 161 vs 162 | rubash-caused: auto-generated _bash-it-component-completion-callback-on-init-aliases missing (mechanism not isolated - marked unverified in issue) |
| comments | #289 (nvm nested-comsub leak evidence), #223 (residual status post-close), #311 (8-shape widening) | - |

## Environment-bound findings (no rubash issue)

1. bash-completion 31 DIFF rows: missing/version-drifting target binaries on Windows,
   winuxsh awk dialect warnings, watch's OSTYPE linux/darwin gate (MINGW64 identity
   is the #154 policy decision), ip/man needing real binaries + /usr config.
2. nvm 11/15 diverged files fail identically under Git Bash 5.3.15 (node.exe
   platform detection in nvm.sh v0.40.8, symlink/hash/MANPATH Windows semantics).
3. Homebrew uninstall.sh platform gate + /usr/bin/sudo; pyenv init - shell-name
   detection ("rubash.exe" - faithful /proc/$PPID/cmdline reading); winuxsh find
   error-prefix wording; docker-binary asymmetry; concourse jq absence (both sides);
   liquidprompt test_git (symlink fixtures) / test_terminal_device (no ttys) /
   test_utils (drvfs cwd, fails on GNU too).
4. Oracle-side infra: GNU bash -i --rcfile for bash-it hangs >25 s/item in this WSL
   instance (full GNU interactive matrix impossible; sampled).

## Positive parity results worth keeping

- bash-it per-component 221/221 clean loads (first time measured item-by-item).
- bash-completion driven matrix: 39/72 queries byte-identical including ssh-keygen
  (59 candidates), tar, gzip, python, gpg (433+), find, lsof, wget classes.
- Homebrew install.sh still byte-clean at HEAD; docker completion (largest static
  completion file, 5598 L) loads + registers cleanly.
- pyenv/rbenv rehash + version management at parity (the old 127-cascade family is
  dead at HEAD).
- sdkman init chain, ohmyzsh bash tools, concourse script family: all load/parse
  parity.

## Artifacts

- Harnesses + verdicts: bc/ (matrix.tsv, drive.sh, run-matrix.sh, verdict.txt,
  rows/), bit/ (run-item.sh, run-rb.sh, sample-*, fnlist-*), lp/ (run-lp*.sh),
  nvm/ (run-*2.sh, index.txt, f26-trace.err), ble/, brew/, pyenv/, sdkman/, docker/,
  omz/, gitres/.
- Issue-isolation probes: p-*.sh + pv-*.sh ladders with {rb,gnu}* outputs.
- Corpora pins: target/ecosweep2-corpus/SOURCES.txt.
