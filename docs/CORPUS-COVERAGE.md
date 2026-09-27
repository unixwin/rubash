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

## 1. modernish use test suite — BLOCKED AT INIT (rubash-caused)

- Source: https://github.com/modernish/modernish @ 63bdae02 (0.17.23-dev),
  run as `bin/modernish --test -q -e`.
- GNU baseline: 389 tests — 384 succeeded, 5 skipped, 0 warnings, 0 xfail,
  0 unexpected failures. (gnu.out/gnu.err)
- rubash: cannot complete initialisation; the .t test bodies are unreachable
  until the blockers below are fixed.
- Blockers, in order:
  1. rubash-caused — rubash#218: unquoted parameter expansion in assignment
     RHS collapses `\\` to `\` (fatal.sh FTL_NOFSPLIT builds its comparison
     string via `t=${#},${1-U},...`; $6 `'\\fo\u\r'` loses its backslashes,
     the case misses both accepted patterns, fatal.sh exits, the comsub
     verification trap yields `fatalbug` instead of `$PPID`,
     `_Msh_initExit "Fatal shell bug(s) detected"` -> exit 128).
     Workaround for harvesting: `MSH_IGNORE_FATAL_BUGS=1`.
  2. env-bound — goodsh.sh requires `$PPID` continuity across an exec'd
     candidate shell; on Windows every native child reports PPID=1, so no
     candidate can ever match `$$` and modernish aborts with "Can't find any
     suitable POSIX-compliant shell!". This cannot pass on Windows with ANY
     engine (probed: `$(exec /d/Git/usr/bin/sh.exe -c 'echo $PPID')` -> 1).
     Harvest workaround: patched copy `target/ecosuite/modernish-harvest`
     accepting the first candidate (GNU on the same patched copy: still
     389/384 green, so the patch is inert for the oracle).
  3. rubash-caused — rubash#219: `{ case...esac; cmd # comment }` spurious
     syntax error; `rubash -n bin/modernish` rejects the whole launcher
     (rc=2) where GNU accepts. Even past blockers 1-2 the launcher refuses
     to parse. This is the current hard stop for the 389-test harvest.
- Verdict: suite NOT runnable under rubash today; 2 rubash-caused bugs filed
  (#218, #219) with minimal reproducers + full reduction chain under
  `target/issue-suites/results/ecosuite/modernish/probes/` (p9, p37 and
  p11a..p45 bisect artifacts).

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

