# ecosuite fixtures (lane wt4/ecosuite, 2026-09-27)

Pinned reproducer fixtures harvested from dedicated mixed evaluation suites.
Oracle: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`), always run from a script
file (`MSYS_NO_PATHCONV=1 wsl bash /mnt/d/<path>`), never `wsl bash -c`.

## Provenance

| Suite | Source | Commit |
| --- | --- | --- |
| modernish | https://github.com/modernish/modernish | 63bdae02eeda8ae2ffb9b517cf26a6f4fd52e180 |
| mvdan/sh corpus (syntax/filetests_test.go `in` strings, extracted) | https://github.com/mvdan/sh | aebdf2b86f56f96eb885260bb03827aa5fc94ec4 |
| nvm fast tests (not pinned here; needs nvm checkout) | https://github.com/nvm-sh/nvm | a885b885fef16fac4bc544188fb25e9e37ae83e8 (v0.40.8) |
| bats-core (not pinned here; needs bats-core checkout) | https://github.com/bats-core/bats-core | 52439ebfac39987dd43e26502aa4aa19f2753d4 |

## modernish/

- `assign_rhs_unquoted_param_collapses_backslashes.sh` — issue rubash#218.
  Expected GNU output: `assign=[a\\b,x\\y]`, `arg=[a\\b,x\\y]`,
  `literal-assign=[\z]`, `quoted-assign=[a\\b]`, `after-setfCu=[a\\b]`.
  rubash at filing: `assign=[a\b,x\y]` (backslash pair collapsed).
- `brace_group_case_trailing_comment_parse.sh` — issue rubash#219.
  `bash -n` rc=0; rubash at filing: rc=2
  `line 2: syntax error: unexpected end of file from '{' command on line 1`.

## mvdan-n/ (16 of 559 snippets diverged on `-n` exit code)

Run: `<shell> -n <file>`; GNU rc vs rubash rc at filing, issue:

| file | GNU rc | rubash rc | issue |
| --- | --- | --- | --- |
| t0026.sh | 0 | 2 | rubash#222 |
| t0068.sh | 2 | 0 | rubash#220 |
| t0142.sh | 2 | 0 | rubash#220 |
| t0143.sh | 2 | 0 | rubash#220 |
| t0144.sh | 2 | 0 | rubash#220 |
| t0164.sh | 0 | 2 | rubash#222 |
| t0169.sh | 2 | 0 | rubash#221 |
| t0173.sh | 2 | 0 | rubash#221 |
| t0286.sh | 0 | 2 | rubash#222 |
| t0382.sh | 2 | 0 | rubash#220 |
| t0383.sh | 0 | 2 | rubash#222 |
| t0411.sh | 0 | 2 | rubash#222 |
| t0415.sh | 2 | 0 | rubash#221 |
| t0459.sh | 1 | 0 | rubash#221 |
| t0525.sh | 2 | 0 | rubash#220 |
| t0526.sh | 2 | 0 | rubash#220 |

## git-contrib/ (rubash#2907)

- `driver.sh` + `make-repos.sh` — chain harness for the REAL
  git-completion.bash / git-prompt.sh. Those two scripts are GPL and
  are **not vendored** (same partition discipline as
  `tests/regression/corpus/PROVENANCE.txt`); the harness discovers
  them on the host (`RUBASH_GIT_CONTRIB_DIR`, else the host git
  installation's shipped pair). `make-repos.sh` builds deterministic
  repositories in eight states (clean / dirty / staged / untracked /
  all+stash / detached / ahead, plus a seeded bare remote);
  `driver.sh` sources the real scripts and drives their live entry
  points (`__git_wrap__git_main` for completion, `__git_ps1` for the
  prompt), each case feeding the next.
- Pin: `tests/issue2907_git_contrib_chain.rs` runs the chain under
  rubash and under a GNU bash oracle over the same repositories and
  requires byte-identical stdout. First run as eco-2907 on master
  `6eb861b8` (git 2.43.0 contrib scripts, GNU bash 5.3.20): chain A
  (16 cases) all SAME; chain B differed on the detached cell only —
  the assignment-carrier leak filed as #3034 (root of closed #3005)
  and fixed by PR #3056; the A15 stderr filename divergence was
  filed as #3035 and fixed by PR #3047.

## mvdan-n re-check (rubash#2907)

The 16 parse fixtures above are re-checked in-repo by
`tests/issue2907_git_contrib_chain.rs` (`-n` exit code per file,
rubash vs GNU bash oracle). This is a re-check of the pinned
reproducers only: whether the full 559-snippet execution
differential is wired into CI or stays a periodic chain is the
decision recorded in rubash#3033, which this pin does not pre-empt.

## bats/

- `zero_pwd_form.sh` — issue rubash#224. Invoke by absolute /d/-style path;
  GNU keeps `$0`/`BASH_SOURCE` in the caller's form; rubash at filing
  rewrites them to `D:/` form while `$PWD` stays `/d/` (mixed path domains).

Raw run artifacts: `target/issue-suites/results/ecosuite/` (modernish/,
mvdan-n/, nvm/, bats-core/).

## Manual-run list (rubash#2910)

Some ecosuite faces cannot be chained in automation — live TUI interaction,
live-session rendering, network-heavy installs. Per the 2026-11-06 ruling
(manual local runs are acceptable coverage when recorded), those faces sit on
one recorded list with a named procedure per face and a run log: the list
lives in the ecosuite warehouse as `MANUAL-RUNS.md` (warehouse-local git,
`ecosuite` commit `483c2bb`), and covers:

- fzf: the real junegunn/fzf `shell/key-bindings.bash` /
  `shell/completion.bash` (never vendored or sourced by any harness) + live
  fzf TUI interaction (Ctrl-R / Ctrl-T / Alt-C / `**` completion).
- ble.sh (pin v0.4.0-devel3): multiline editing + completion rendering in a
  live session (`ble-attach` is unreliable in automation).
- oh-my-niu bundle: end-to-end exercise from an installed location + the
  theme-gallery visual pass.
- nvm (pin v0.40.8): `test/slow` + `test/installation_iojs` (real-network
  runs; never run on any shell so far).

A manual pass counts as coverage only when it is logged in that document
(date, operator, binary/version, result). As of the list's creation
(2026-11-08), no manual runs are recorded yet for any of the four faces.
