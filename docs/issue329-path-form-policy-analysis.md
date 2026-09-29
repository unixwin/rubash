# Issue #329 policy analysis — POSIX vs drive-form path rendering (CAPTAIN DECISION)

Author: pbbfix lane (wt14/pbbfix), 2026-09-28. Per the lane charter this
issue is ANALYSIS ONLY — no code changed. Evidence probes:
`target/probe/p329*.sh` and the runs below. GNU oracle: WSL GNU Bash
5.3.0 (`/usr/local/bin/bash`).

## 1. What actually happens (measured, current master + pbbfix commits)

The issue's title mechanism — "env-inherited POSIX paths re-rendered by
rubash to D:/ drive form" — decomposes into FOUR distinct sources, only
some of them rubash:

**(a) The parent-layer conversion — NOT rubash.** A POSIX-form value
exported in a winuxsh session reaches EVERY Windows child already in
drive form; rubash imports it verbatim (GNU-faithful):

```sh
export P329TEST=/d/some/path/value
cmd /c echo %P329TEST%        # -> D:/some/path/value
python -c 'import os; print(os.environ["P329TEST"])'  # -> D:/some/path/value
rubash.exe -c 'echo $P329TEST'                          # -> D:/some/path/value
```

PowerShell (no winuxsh in between) setting `HOME=/d/...` directly:
`rubash.exe -c 'echo $HOME'` → `/d/...` — rubash does NOT re-render on
import. The ecosweep3 nvm/webi flows ran under the winuxsh-carried
harness (`inst/run-rub.sh`: `export HOME="$w/home"` with `$w=/d/...`),
so the `sed "s:^$HOME:..."` colon collision came from the PARENT's
conversion, before rubash ever saw the value.

**(b) rubash's OWN inconsistent emissions.** Within one shell, the same
directory renders in BOTH forms:

```sh
cd /d/repo/rubash-wt-pbbfix/target/probe
echo "$PWD"          # /d/repo/rubash-wt-pbbfix/target/probe   (POSIX)
echo "$(pwd)"        # D:/repo/rubash-wt-pbbfix/target/probe   (drive!)  <- ecosweep2 pitfall
echo "$OLDPWD"       # D:/repo/rubash (inherited Windows form, kept verbatim)
unset TMPDIR; rubash -c 'echo $TMPDIR'
                     # D:\repo\...\probe\target (backslash Windows form, self-injected)
$0 with a full path  # D:/ form (issue #224 history)
```

**(c) GNU baseline.** GNU keeps inherited env values verbatim (no path
semantics at import), and its own `pwd`/`$PWD`/`$0` are POSIX-form. GNU
never faces (a) because Linux parents don't emit drive-form values.

## 2. Blast radius

- nvm install.sh `sed "s:^$HOME:$HOME:"` → `export NVM_DIR=""` (issue
  evidence): source (a) — the harness environment, i.e. the winuxsh
  export boundary, not rubash.
- webi bootstrap `sed "s:^${my_rel}:~:"` death: same class (a)/(b).
- FFmpeg configure TMPDIR corollary: source (b) — the self-injected
  default TMPDIR is BACKSLASH Windows form; an explicitly exported
  POSIX TMPDIR is converted by (a) to `D:/` forward-slash form, which
  survives configure's eval handling (ecosweep3 workaround).
- pbb/ls/glob phantom-dir noise: was #328 (fixed this lane).
- Same-form inconsistency (`$PWD` /d/ vs `$(pwd)` D:/): source (b) —
  pure rubash, breaks scripts that string-compare the two.

## 3. Options for the captain

**Option A — normalize rubash's OWN emissions to POSIX form (bounded).**
`$(pwd)`-in-comsub, the self-injected TMPDIR default, full-path `$0`,
initial `PWD` derivation: all render `/d/...` (the form `$PWD` already
uses). No env-import change, no GNU divergence; kills the
internal-consistency bugs and the FFmpeg TMPDIR corollary. Est: small,
each site has a clear owner (executor/path.rs render helpers).
RECOMMENDED as the rubash-side fix.

**Option B — also re-render INHERITED drive-form values (the issue's
suggestion: POSIX for shell-visible values, convert back at
exec/CreateProcess boundaries).** Fixes the nvm/webi flows regardless of
parent, BUT: (1) diverges from GNU's verbatim-import semantics — GNU
would show `D:/x` if given `D:/x`; (2) needs a path-likeness heuristic
(`X:/`-prefixed values) — false positives mangle non-path data; (3) a
value converted at import must be converted BACK for every external
child (every exec site, plus files passed to Windows tools) — a new
invariant across the whole executor. High risk, medium reward; the
false-negative class (a value that LOOKS like a path) is silent data
corruption.

**Option C — upstream the parent-layer conversion.** File at
unixwin/Winuxsh (unixwin-winuxsh repo): exported POSIX-form path-like
values being converted to drive form when Windows children are spawned.
That conversion is what actually broke nvm/webi in the ecosweep3 runs.
rubash stays GNU-faithful. Pair with Option A.

**Recommendation: A + C.** A makes rubash internally consistent and
POSIX-first for its own values; C fixes the actual observed breaker at
its owner. B only if the captain weighs installer-corpus parity above
GNU-import fidelity and accepts the re-conversion invariant.

## 4. Verification bar for whichever option

Any change must run: the nvm install.sh sandboxed flow (NVM_DIR
non-empty, no sed error), the webi bootstrap to completion, FFmpeg
configure without the TMPDIR workaround, `[[ $PWD == $(pwd) ]]` true in
a /d/ cwd, and the full issue-suite battery (path rendering feeds
COMPATIBILITY-STATUS lines). Baselines via `scripts/true-baseline.sh`.
