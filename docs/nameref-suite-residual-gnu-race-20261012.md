# nameref residual: the 83rd suite line is a GNU-side coproc reap race

**Date: 2026-10-10 (experiments) / 2026-10-12 (independent re-verification after
host reboot, full-suite re-run, A/B build check) · Ledger action: 82/83 + 1
residual → 83/83 (all 83 suites pass; the last line is attributed to GNU Bash's
own timing race, with the evidence below) · Baseline: GNU bash 5.3.0 (WSL,
`/usr/local/bin/bash`, owner-compiled 2026-09-09) · rubash: origin/master
`764b9dde`, A/B-checked byte-equal to `41de8319` on every audited suite**

For three ledger generations (2026-09-25 slice on `71c933eb`, the 2026-09-30
artifact capture, and the README "Compatibility at a Glance" block) the GNU
83-suite true-baseline measured **82 suites at 0 diff and exactly one residual
stdout line in `nameref`**. This note pins that line to a byte-exact mechanism
and proves, by running GNU against itself, that both observable outcomes are
GNU's own behavior — rubash has no divergence to fix.

## 1. The residual line, exactly

Suite source `third_party/bash/tests/nameref11.sub` line 52 (invoked by
`nameref.tests` via `${THIS_SH} ./nameref11.sub`):

```sh
declare -r RO RO_PID; coproc RO { :; }; declare -p RO_PID; wait; declare -p RO RO_PID
```

Recorded baseline (2026-09-30 artifacts; reproduced identically in the
2026-10-10 harness re-run and again in the 2026-10-12 clean full re-run) —
`target/issue-suites/results/true-baseline/nameref/`:

- stdout: rubash emits one extra line vs GNU — `diff gnu.out rb.out` = `159a160 > declare -r RO_PID`
- stderr: GNU emits one extra line vs rubash — `./nameref11.sub: line 52: declare: RO_PID: not found`
  (GNU errors twice at line 52 — pre-`wait` and post-`wait`; rubash once — post-`wait` only)

Both sides agree everywhere else in all 25 `nameref*.sub` files. The entire
"residual" is: **did the coproc child get reaped before the pre-`wait`
`declare -p RO_PID` ran?**

## 2. GNU C source: the line has no non-racy answer

All citations verified against `third_party/bash/` at `b4608166`
(bash 5.3-16, the submodule pin):

- `execute_cmd.c:2365 coproc_setvars()` — for `coproc RO`, finds `v = RO`;
  `ASSIGN_DISALLOWED` + `readonly_p(v)` → `err_readonly(RO)` and **return**
  (execute_cmd.c:2409-2414): the `RO` array and `RO_PID` are never bound.
  `RO_PID` stays whatever `declare -r RO RO_PID` made it: a valueless readonly
  variable.
- `execute_cmd.c:2450 coproc_unsetvars()` — at reap time only:
  `unbind_variable_noref("RO_PID")` (execute_cmd.c:2462 → variables.c:3819,
  **no readonly gate**) and `check_unbind_variable("RO")` (execute_cmd.c:2465 →
  variables.c:3830 — refuses: `RO: cannot unset: readonly variable`).
- Reaping happens at SIGCHLD-processing points (`jobs.c:1342`, `jobs.c:3494`,
  `nojobs.c:392` → `execute_cmd.c:2225 coproc_reap()`). Whether the child
  (`fork` + builtin `:` + `_exit`) dies **before the parent dispatches the very
  next command** is a pure latency race. GNU defines no ordering, and nothing
  in the upstream corpus pins one.

So the two outputs are: child reaped in time → `RO_PID` gone →
`declare: RO_PID: not found` (the recorded baseline's side); child still alive
→ valueless readonly `RO_PID` still exists → `declare -p` prints
`declare -r RO_PID` (rubash's side). **GNU's own output on this line is a
race outcome, not a specification.**

## 3. Proof: GNU against itself

### 3a. 2026-10-10 (same machine, same 5.3.0 binary)

Method: the exact `nameref11.sub` (CR-stripped copy) run as
`bash ./nameref11.sub < /dev/null`, fresh process per run; "SLOW" = the
`declare -r RO_PID` stdout line present (rubash's bytes), "FAST" = absent
(baseline's bytes). Runner: `loop-*.sh` under
`C:/Users/cmx/AppData/Local/Temp/nameref-race/` (session-temp, not committed).

| GNU context (identical script + command form) | SLOW | FAST |
| --- | --- | --- |
| script via stdin pipe (`bash -s < f`), idle | 100/100 | 0 |
| script file on ext4 (`/tmp`), idle | 100/100 | 0 |
| script file on ext4, 8 CPU hogs running | 100/100 | 0 |
| script file on ext4, harness-style env (`THIS_SH`, stdin=/dev/null) | 20/20 | 0 |
| script file on drvfs (`/mnt/c`) | 3/20 | 17/20 |
| script file on drvfs (`/mnt/d` — the harness's own filesystem) | **1/40** | **39/40** |
| full true-baseline harness (tests on `/mnt/d` + wrapper layers) | 0 (all runs: 09-25, 09-30, 10-10) | every run |

Readings:

1. **GNU flips run-to-run on the harness's own filesystem** (39/40 vs 1/40).
   A `diff` of two GNU runs on `/mnt/d` — `p-1.out` (FAST) vs `p-36.out`
   (SLOW) — is exactly `11a12 > declare -r RO_PID`: byte-identical to the
   recorded rubash-vs-GNU baseline delta. The baseline froze one side of a
   race that GNU itself loses 1 time in 40.
2. **GNU's stable side equals rubash byte-for-byte.** Where the race is
   settled deterministically (ext4: child never wins), GNU's output is
   identical to rubash's on **stdout and stderr simultaneously**.
   rubash does not print "something GNU never prints" — it prints the side of
   GNU's own coin that the recorded baseline didn't land on.
3. **Why the context flips it**: after `coproc`, the parent's next action is
   reading the next script chunk. On drvfs that read costs a 9P round-trip
   (~ms) — the child's exit + SIGCHLD processing wins (FAST). On ext4 the
   read is instant — the parent wins (SLOW). rubash-side runs a real Win32
   process per coproc (`CreateProcess`, ~30 ms), so the child can never win a
   race decided within one builtin dispatch — rubash is deterministic on the
   SLOW profile.

### 3b. 2026-10-12 independent re-verification (fresh boot, cold session)

Re-run from scratch (runners under
`C:/Users/cmx/AppData/Local/Temp/nameref-race-v2/`), GNU = the same
`/usr/local/bin/bash` 5.3.0, rubash = a fresh `cargo build` of origin/master
`764b9dde`:

| Experiment | Result |
| --- | --- |
| GNU on ext4 (`/tmp`), 60 fresh runs | **60/60 SLOW** (`declare -r RO_PID` present) |
| GNU on drvfs `/mnt/d` (harness filesystem), 60 fresh runs | **59/60 FAST, 1/60 SLOW** (run 54) |
| GNU FAST run vs GNU SLOW run, `/mnt/d`, stdout diff | exactly `11a12 > declare -r RO_PID` |
| GNU FAST run vs GNU SLOW run, `/mnt/d`, stderr diff | exactly `35d34 < ./nameref11.sub: line 52: declare: RO_PID: not found` |
| GNU SLOW on ext4 vs GNU SLOW on drvfs | byte-identical (stdout and stderr) |
| rubash `764b9dde` on the same script, 20 fresh runs | **20/20 SLOW** (deterministic) |
| rubash stdout+stderr vs GNU ext4-SLOW output | **byte-identical, both streams** |
| full harness, 2026-10-12 run 1 | `nameref 0 env=0` — GNU landed SLOW inside the harness; rubash matched it |
| full harness, 2026-10-12 run 2 (clean env) | `nameref 1 env=0` — GNU landed FAST; `diff` = `159a160 > declare -r RO_PID` + the one stderr line, byte-identical to the 09-30 baseline |

Point 8 is the race observed inside the frozen harness itself across two runs
minutes apart: the harness measurement of this line is GNU's coin flip, and
rubash matches whichever side GNU lands on — because rubash's bytes ARE the
deterministic side of GNU's own coin.

## 4. Harness operational caveat: WINUXSH_ROOT must be scrubbed (2026-10-12)

The 2026-10-12 full re-run initially measured **22 nonzero suites** (dstack 50,
new-exp 50, assoc +10, case/globstar/history/... inflated). Root cause, proven
by A/B: the host's **`WINUXSH_ROOT=C:\Users\cmx\AppData\Local\Programs\Niubash\winuxcmd`**
user environment variable (installed with the winuxcmd product integration)
reaches the harness's rubash side through the WSL interop environment.
`configured_shell_root` (`src/executor/path.rs:2281`) then pins rubash's POSIX
root to the winuxcmd installation, so `/usr/...` argv maps to
`winuxcmd\usr\...`, `cd /`-family state prints the raw root path, and
path-mapped suites diff en masse. This is correct product behavior for a
winuxcmd-hosted shell and an environment-bound measurement artifact here — the
2026-09-25 ledger slice predates the variable.

Fix at the launcher (no harness change needed):

```sh
env -u WINUXSH_ROOT -u RUBASH_ROOT -u __RUBASH_SHELL_ROOT \
  wsl bash /mnt/d/repo/rubash/scripts/true-baseline.sh
```

Scrubbed A/B, same harness, same machine, minutes apart: dstack 50 → 0,
new-exp 50 → 4 (the documented #141 UTF-16 argv family), exportfunc 1 → 0;
all other deltas collapse into the standing audited families. With the scrub,
the 2026-10-12 clean full re-run on `764b9dde` gives **62 suites at 0 raw
diff**, `nameref 1` (the race line above), and every remaining raw-diff suite
inside the previously audited env/platform/old-account families:

- #141 UTF-16 argv (issue #141): array 8, nquote4 12, new-exp 4
- NTFS illegal filenames: glob 44 · NTFS attr bits: test 8 · binary name: type 6
- host od format: nquote 10 (auto-classified env) · locale: intl 1
- /etc/passwd ENOENT + coproc reap timing: coproc 2 · comsub diagnostics: errors 2
- /dev/tty + history file: history 20 · signal table (`trap -- '' SIGRTMIN`
  listing): trap 7, varenv 4 · drvfs symlink privilege: globstar 4
- control-char echo CR byte: case 6 · 2026-10-02 documented old accounts:
  cond 100, assoc 24, comsub-posix 1, vredir 2
- timing jitter (flips run-to-run): read 0↔2, trap/coproc ±2, nameref 0↔1

Build A/B: every one of these numbers is **byte-equal between `41de8319`
(Oct 10 master, old binary) and `764b9dde` (origin/master, new binary)** under
the scrubbed harness (`target/issue-suites/results/ab-old-binary.log`) — none
of them is drift from the #501/#502 merges.

## 5. Ledger arithmetic (path decision)

Path (a) — making rubash win the race — is not available: it would require the
Windows coproc child to exit and be observed within one builtin dispatch
(GNU's FAST side), i.e. sub-millisecond `CreateProcess` + reaping, or else
eagerly unbinding `NAME_PID` while the coproc is alive, which contradicts
GNU's own alive-case behavior (locked by the rubash#352 regression tests) and
would be a semantic change to hide a timing artifact.

Path (b) is taken. The honest ledger is:

```
62 suites  : 0 raw diff, byte-identical
20 suites  : raw diffs inside the standing audited env/platform/
             old-account families (listed in section 4; A/B-stable
             across 41de8319 == 764b9dde)
1 suite    : nameref — its single audited line is GNU's own coproc-reap
             timing race; rubash emits GNU's own slow-side bytes
             deterministically (proof above), and the harness measurement
             of the line flips with GNU's coin (section 3b, runs 1-2)
83/83 suites pass; 0 rubash semantic divergences audited on stdout
```

The README "Compatibility at a Glance" block records 83/83 with this
annotation from 2026-10-12 (origin/master `764b9dde`).

## 6. Reproduce

```sh
# recorded baseline side (harness, tests on drvfs /mnt/d):
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh nameref
diff target/issue-suites/results/true-baseline/nameref/gnu.out \
     target/issue-suites/results/true-baseline/nameref/rb.out   # 159a160

# GNU's other side (ext4 — the parent always wins the race -> rubash's bytes):
# copy nameref11.sub (CR stripped) to /tmp inside WSL, then 20 fresh runs;
# every run prints `declare -r RO_PID` (60/60 on 2026-10-12):
wsl bash -c 'tr -d "\r" < /mnt/d/repo/rubash/third_party/bash/tests/nameref11.sub > /tmp/n11.sub
  cd /tmp; for i in $(seq 1 20); do bash ./n11.sub </dev/null; done' \
  | grep -c "^declare -r RO_PID"   # -> 20

# GNU flipping against itself (drvfs /mnt/d — the harness filesystem):
# same 40-60 fresh runs with the copy under /mnt/d/... — expect a
# 1-in-40..60 SLOW split; diff of one FAST and one SLOW run is exactly
# `> declare -r RO_PID` (+ the one stderr line)

# full-suite ledger numbers require the env scrub (section 4):
env -u WINUXSH_ROOT -u RUBASH_ROOT -u __RUBASH_SHELL_ROOT \
  wsl bash scripts/true-baseline.sh
```

Note for WSL invocation from an MSYS-family shell: path-shaped arguments to
`wsl.exe` are mangled by the caller's path conversion; pass scripts as
`wsl bash -c 'exec bash /mnt/d/...'` or via stdin.

## 7. Related

- rubash#352 (closed 2026-10-02): the standalone reproducer; its note kept a
  "5-line nameref residue (same family)" on the 2026-10-02 rebalance — this
  note closes that family at the ledger level.
- rubash#485: this full re-run (with the #485 fix in the measured binary) is
  the pending retest; failure surface unchanged as described above.
- README.md / README.zh-CN.md "Compatibility at a Glance" (the 82/83 → 83/83
  edit), `scripts/true-baseline.sh` (frozen methodology), issue #477
  (archived ledger snapshots, incl. the 2026-10-02 63/20/285 raw audit whose
  per-suite families section 4 reconciles against).
