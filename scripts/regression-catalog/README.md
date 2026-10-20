# Per-issue regression catalogue

One case per filed issue: a small shell body, run twice — once under the GNU
oracle, once under the arm — and compared on exit status, stdout and stderr.

This is a **guard**, not a scoreboard.  It exists so that a fix cannot silently
regress and so that a stale binary cannot be mistaken for a fresh measurement.

## Run it

```sh
# from the repository root, with a current build in target/release/
python3 scripts/regression-catalog/runner.py

# a different arm, or a single case, or an older checkout on purpose
python3 scripts/regression-catalog/runner.py --arm target/debug/rubash.exe
python3 scripts/regression-catalog/runner.py --only issue-743,issue-753
python3 scripts/regression-catalog/runner.py --allow-stale      # archaeology only
```

Requirements: **WSL GNU bash 5.3.0** (`/usr/local/bin/bash`) as the oracle, and
Python 3 on the Windows side (the same shape as `scripts/fast-path-differential.py`).
The runner invokes `wsl.exe` itself and never uses Git Bash or the PATH shim as
the oracle.

Artifacts land under `target/issue-suites/results/regression-catalog/`:
`summary.tsv`, and per case `out/<case>/{gnu,arm}.{out,err,rc}` plus `diff.txt`.

## Expectations and the two alarms

`manifest.tsv` has one row per case:

| column | meaning |
|---|---|
| `case` | basename of `cases/<case>.sh` |
| `issue` | the issue the case pins |
| `expected` | `diverges` (known-bad) or `must-match` (fixed, must never diverge again) |
| `note` | one line of context |

A run reports:

* **ALARM** — a `must-match` case no longer matches.  A regression; the exit
  status is 1.
* **PROMOTE** — a `diverges` case now matches.  The fix landed; flip that row to
  `must-match` so it can never regress.
* **UNSCORED** — a case file with no manifest row.  It runs but is never
  scored, so the ratchet cannot see it: the exit status is 2.  (Four files
  shipped this way before the check existed.)
* **DANGLING** — a manifest row with no case file (full runs only).  Also
  exit 2.

Adding a case for a new issue: drop `cases/<case>.sh` (self-contained, no
positional arguments, no shared fixtures), add a manifest row with
`expected=diverges`, and confirm the case reproduces against the current build.

## The stale-build gate

```
arm      : target/release/rubash.exe
arm built: 2026-10-17 14:25
HEAD     : b5793580  committed 2026-10-17 14:25
GATE     : ok (arm is not older than HEAD)
```

If the arm predates HEAD the runner exits 2 rather than printing a table, because
a ledger from a stale binary is worse than no ledger.  Three reports in this
project have already been filed against builds older than their fixes; the gate
exists to make that impossible to do by accident.

## Invocation rules (each one learned by breaking it)

* the oracle is always WSL GNU bash; never Git Bash, never the PATH shim;
* every arm is bounded and its whole process tree is killed on timeout, and an
  arm that does not reach the end of its own script is `INCOMPLETE` — never a
  pass (this is how `issue-738` and `issue-758` are detected at all);
* each arm gets its own staged work dir (`out/<case>/{arm,gnu}/`); fixtures
  one arm leaves behind cannot contaminate the other;
* no positional argument is ever passed to an arm — the host rewrites them, so
  everything a case needs lives in the case file;
* line-ending normalisation **skips binaries**: rewriting CRLF inside a helper
  EXE produces `ERROR_BAD_EXE_FORMAT` (os error 216) and then every
  helper-dependent case fails for *both* arms, which looks exactly like a
  divergence and is not one;
* comparisons are made by the runner on the captured files, and only the
  harness's own scratch paths (in every spelling: `C:\…`, `C:/…`, `/c/…`,
  `/mnt/c/…`) and the per-arm `HOME` are normalised.  Nothing about the shells'
  own path rendering is normalised.

## Scope

Behavioural cases only.  Source-level/inventory items (`#713`, `#716`, `#727`,
`#728`, `#729`, `#767`, `#768`) are not cases; `#767` in particular belongs in
the harness rather than in a case file, and the rest are re-checkable with the
commands quoted in each issue.

Not wired into CI yet: no workflow on this repository currently runs Python.  If
the gate below is wanted in CI, the honest form is a job that builds the arm,
runs this runner, and fails on `ALARM` — the runner already exits 1 for that.

First recorded run: 27 cases, arm `b5793580`, `ok=27 alarms=0 newly-fixed=0`
(posted on #746).

## The #750-#810 wave (first batch of fixed-issue rows, rubash#746)

Every fixed issue of the wave carries ONE catalogue line.  Where the fix face
is behavioural it is a case above (flip verified on the arm named in the last
run below); where the fix lives in harness/Rust-test territory it is a pointer
to the pin, per the Scope rule.

| issue | fix surface | catalogue line |
|---|---|---|
| #714 | `read -n/-N` count decoded units | `issue-714` (case, `must-match`) |
| #723 | pipeline while-read consumer stall | `issue-723` (case, `must-match`) |
| #726 | definition-level dup splice; `exec` group-order escape | `issue-726` (case, `must-match`) |
| #733 | compound-assignment quote leak | `issue-733` (case, promoted 2026-10-19) |
| #735 | PUA/marker collision | `issue-735` (case, kept `diverges`: `${x@Q}` byte face — follow-up #794/#796) |
| #737 | kill reaches background children | `issue-737` (case, kept `diverges`: Killed-notification wording — follow-up #794) |
| #743 | partially-quoted word field splitting | `issue-743` (case, promoted; rework follow-up #795) |
| #753 | failglob abort scope | `issue-753` (case, promoted; also `tests/regression.rs` `issue_753`) |
| #755 | scalar re-parsed as array by `${x[@]}` | `issue-755` (case, promoted) |
| #756 | `history -w` with HISTFILE unset | `issue-756` (case, promoted; wording face moved to #551) |
| #758 | malformed expansion swallowed | `issue-758` (case, promoted) |
| #761 | unbounded producer + shell-side tail drain | `issue-761` + `issue-761b` (cases, promoted) |
| #772 | non-UTF-8 `$(cat)` / `${x:off:len}` opacity | `loc-l1-substring` (promoted); `loc-l2`/`loc-l5` kept `diverges` (residual faces) |
| #776 | `read -N` keeps embedded newline | `issue-776` (case; also `tests/regression.rs` `issue_776`) |
| #780 | argument-less `-O`/`+O` shopt listing | pointer: `tests/regression.rs` `issue_780` (startup-flag face, not expressible in a case body) |
| #788 | compound operand sets the array attribute | `issue-788` (case; also `tests/regression.rs` `issue_788`) |
| #797 | sourced bash-preexec DEBUG-trap assembly | `issue-797` (case) |
| #798 | `exit` inside source terminates | `issue-798` (case) |
| #799 | command-name stem byte-exact | pointer: `tests/issue799_command_name_case_regressions.rs` (r/R cannot coexist on NTFS and WSL DrvFs is itself case-insensitive, so no differential case is expressible) |
| #803 | nquote snapshot vs autocrlf | pointer: the snapshot runner's CRLF-normalizing copy predicate (`tests/regression.rs` `run_suite_snapshot`) — a harness face, not a behaviour |

## The 2026-10-21 batch (rubash#746 continued)

Second wave: the early-sweep rubash issues that predate the catalogue
(#699/#700/#701), a set of still-open behavioural issues, the fixed faces of
the #826-#846 wave, and the four case files that had landed without manifest
rows.  Diagnostic wording is deliberately not pinned — cases that exercise a
diagnostic route it to a file and assert only emptiness (the #756 precedent);
where residual faces were found on closed issues they were filed separately so
a `diverges` row points at live work.

| issue | state | catalogue line |
|---|---|---|
| #699 | closed/fixed | `issue-699` (must-match): redirect order, write-error routing, closed-stderr rc, subshell cd diag, bare-cat self-redirect |
| #700 | closed; residual #938 | `issue-700` (diverges): file-stdin form fixed, pipe-context `/dev/stdin` + `<&0` read empty |
| #701 | closed/fixed | `issue-701` (must-match): `/dev/fd/N` + EOF-without-newline rc family |
| #830 | closed/fixed | `issue-830` (must-match): `popd` chdir |
| #832 | closed/fixed | `issue-832` (must-match): `$(echo 's')` CWD-independence |
| #840 | open | `issue-840` (diverges): declare -p E400 leak, `__RUBASH_CSB1_` re-decode; `\uE30D` assign is the parity control |
| #846 | closed; residual #940 | `issue-846` (diverges): top-level rc fixed, diagnostic-bypass + nested-`!` abort remain |
| #856 | open | `issue-856` (diverges): literal-PUA spelling eaten at the output boundary; `$'\uE000'` escape control matches |
| #860/#861 | closed/fixed (PR #883) | manifest rows added — the case files shipped unscored |
| #868/#876 | closed/fixed | manifest rows added — the case files shipped unscored |
| #871 | closed; residual matches | `issue-871` (must-match): `infin`/`nann`/`infx`/`nanx` print the value and emit `invalid number` + rc 1 — byte-identical to GNU (promoted from diverges per PR #944 review) |
| #879 | open; case faces match | `issue-879` (must-match): `%f 1e999`/`%a 1e999` long-double expansion + glibc `%La` mantissa style — byte-identical to GNU (promoted from diverges per #949) |

Residuals found while building this batch and filed for the record:
#938 (read empty via `</dev/stdin` / `<&0` in pipeline context) and #940
(redirect-failure diagnostic bypasses the ordered fd table on `()`/simple
faces; `!`+compound+redirect failure inside a group aborts the script).
#871's prefix-subclass residual is noted on its own row rather than
reopened; that face has since been verified byte-identical to GNU and
its row promoted to must-match (PR #944 rework).

Not pinned by this batch: #705 (needs argv/stdin at the launcher — the runner
deliberately passes a script file with fd 0 on /dev/null) and the static/
inventory issues (#716, #727, #728, #729) stay non-cases per the Scope rule.
The niubash and WinuxCmd tables of #746 live outside this repo's arm.

## Wiring with fast-path-differential.py (#706)

`scripts/fast-path-differential.py` is the #706 harness skeleton; this
catalogue is its generalization past fast paths.  The contract between them:

* case ids trace to issues everywhere: the differential's `known=` notes name
  `rubash#NNN`, and catalogue ids are `<case>` rows whose manifest line names
  the issue — a failing name lands on the issue either way;
* a fast-path known-divergence that starts passing must be PROMOTED out of the
  differential (delete the `known` note) and, when the fix has an issue, given
  a `must-match` row here — the two lists never both carry a fixed face;
* families beyond the fast paths (parser, expansion, fd, jobs, harvest) live
  ONLY here; the differential keeps no second copy of them.
