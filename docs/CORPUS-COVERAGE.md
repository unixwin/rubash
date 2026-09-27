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
