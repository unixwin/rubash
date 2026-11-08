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
| bash-completion | https://github.com/scop/bash-completion | tag `2.14.0` (commit 0543d1a28ce3d36741675c7ef6da7c2286288f3e) |

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

## bats/

- `zero_pwd_form.sh` — issue rubash#224. Invoke by absolute /d/-style path;
  GNU keeps `$0`/`BASH_SOURCE` in the caller's form; rubash at filing
  rewrites them to `D:/` form while `$PWD` stays `/d/` (mixed path domains).

## bash-completion/

- `bash_completion-2.14.0` — the real bash-completion **2.14.0**
  distribution file itself (3,349 lines, 122,328 bytes), pinned for
  rubash#2900 as the regression gate on rubash#2876.
  Source: `bash_completion` at tag `2.14.0` of
  https://github.com/scop/bash-completion (commit
  0543d1a28ce3d36741675c7ef6da7c2286288f3e).
  sha256 `859ddde49ce594914f4c4ea108aed73d834c51f12839f36135e319b5f86d0abc`
  (byte-identical to the upstream tag file; re-verified 2026-11-08).
  Key lines in the pinned file: line 45 `shopt -s extglob progcomp`;
  `_comp_expand_glob` defined at line 335, its `eval -- "$1=($2)"` at
  line 356 (the fallback `eval -- "$1=()"` at line 355; the distro copy
  in the original user report numbers the eval 358);
  `_comp_backup_glob` at line 1947; the caller
  `_comp__init_set_up_service_completions` is defined at line 2018 and
  invoked at line 2031 — it runs at source time against the host's real
  SysV init dirs and then unsets itself.
  Driven end-to-end by `tests/issue2900_bash_completion_chain.rs`:
  source the file, assert the post-source state, call the real
  `_comp_expand_glob` with the caller's exact argument shape against a
  fixture init dir, and feed the result through the caller's
  executability filter into `complete -F` registrations.

Raw run artifacts: `target/issue-suites/results/ecosuite/` (modernish/,
mvdan-n/, nvm/, bats-core/).
