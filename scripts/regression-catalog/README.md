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
* each arm builds its fixtures in its own work dir; nothing is shared;
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
