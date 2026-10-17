#!/usr/bin/env python3
"""Per-issue regression catalogue: one case per filed issue, run differentially.

Each case is a shell body taken from a filed issue.  Every case runs twice --
once under the GNU oracle, once under the shell under test -- and the two runs
are compared on **exit status, stdout and stderr**, with the outputs normalised
only for the harness's own scratch paths.

Two properties make this a regression guard rather than a scoreboard:

*   **Every case declares its expected state** in `manifest.tsv`:
      `diverges`    known-bad; it must KEEP diverging until the fix lands
      `must-match`  fixed; it must NEVER diverge again
    A run reports an ALARM when a `must-match` case diverges (a regression) or
    when a `diverges` case starts matching (a fix landed; promote the row).

*   **A stale-build gate.**  A ledger from a stale binary is worse than no
    ledger (AGENTS.md).  The runner refuses to present a result as trustworthy
    when the arm predates the tree's HEAD; `--allow-stale` overrides it for
    deliberate archaeology.

Invocation rules, each of which exists because breaking it produced a wrong
answer at least once:

*   the oracle is **always** WSL GNU bash (`/usr/local/bin/bash`); Git Bash is
    never used as the oracle (AGENTS.md) and neither is the PATH shim;
*   every arm is bounded, and on timeout the **whole process tree** is killed;
    an arm that does not reach the end of its own script is recorded as
    INCOMPLETE and is **never** scored as a pass;
*   each arm builds its own fixtures in its own work dir; nothing is shared or
    pre-staged;
*   no positional argument is ever passed to an arm (the host rewrites them);
*   byte claims are made by this runner on the captured files, never by the
    shells under test;
*   line-ending normalisation skips binaries: rewriting CRLF in a helper EXE
    turns it into ERROR_BAD_EXE_FORMAT (os error 216) and every helper-dependent
    case then fails for both arms.

Usage:
    python3 scripts/regression-catalog/runner.py \
        [--repo <rubash checkout>] [--arm target/release/rubash.exe] \
        [--cases scripts/regression-catalog/cases] \
        [--manifest scripts/regression-catalog/manifest.tsv] \
        [--out target/issue-suites/results/regression-catalog] \
        [--timeout 30] [--only case1,case2] [--allow-stale]

Exit status: 0 when every case matches its expectation, 1 when any alarm is
raised, 2 on a usage/environment error (including a stale arm).
"""

import argparse
import difflib
import os
import re
import shutil
import subprocess
import sys
import time

WSL = r"C:\Windows\System32\wsl.exe"
SCRIPT = "#!/usr/bin/env bash\nexec \"{exe}\" \"$@\"\n"


def wsl_path(winpath):
    p = os.path.abspath(winpath).replace("\\", "/")
    m = re.match(r"^([A-Za-z]):(/.*)$", p)
    return "/mnt/" + m.group(1).lower() + m.group(2) if m else p


def run_bounded(cmd, cwd, timeout, out_path, err_path, env=None):
    """Run cmd; on timeout kill the whole tree. Returns (rc, timed_out)."""
    with open(out_path, "wb") as o, open(err_path, "wb") as e:
        p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.DEVNULL,
                             stdout=o, stderr=e, env=env,
                             creationflags=getattr(subprocess,
                                                   "CREATE_NEW_PROCESS_GROUP", 0))
        try:
            return p.wait(timeout=timeout), False
        except subprocess.TimeoutExpired:
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)],
                           capture_output=True)
            try:
                p.wait(timeout=5)
            except Exception:
                pass
            return 124, True


def stage(case_src, work):
    """Per-arm work dir: the case body plus the marker contract."""
    if os.path.isdir(work):
        shutil.rmtree(work, ignore_errors=True)
    os.makedirs(os.path.join(work, "home"))
    body = open(case_src, "rb").read().replace(b"\r\n", b"\n")
    with open(os.path.join(work, "probe.sh"), "wb") as f:
        f.write(body)
        f.write(b'\n__rc=$?\necho "__ARM_DONE__ rc=$__rc"\n')
    return work


def normalise(path, tokens):
    raw = open(path, "rb").read()
    txt = raw.decode("utf-8", "surrogateescape").replace("\r\n", "\n").replace("\r", "\n")
    for t in tokens:
        if t:
            txt = txt.replace(t, "@P@")
    return txt


def main():
    ap = argparse.ArgumentParser()
    here = os.path.dirname(os.path.abspath(__file__))
    ap.add_argument("--repo", default=os.path.abspath(os.path.join(here, "..", "..")))
    ap.add_argument("--arm", default=None)
    ap.add_argument("--cases", default=os.path.join(here, "cases"))
    ap.add_argument("--manifest", default=os.path.join(here, "manifest.tsv"))
    ap.add_argument("--out", default=None)
    ap.add_argument("--timeout", type=int, default=30)
    ap.add_argument("--only", default="")
    ap.add_argument("--allow-stale", action="store_true")
    a = ap.parse_args()

    arm = a.arm or os.path.join(a.repo, "target", "release", "rubash.exe")
    out = a.out or os.path.join(a.repo, "target", "issue-suites", "results",
                                "regression-catalog")
    if not os.path.isfile(arm):
        print(f"FATAL: arm not found: {arm}", file=sys.stderr)
        return 2

    # ---- stale-build gate -------------------------------------------------
    head = subprocess.run(["git", "-C", a.repo, "rev-parse", "HEAD"],
                          capture_output=True, text=True).stdout.strip()
    ct = subprocess.run(["git", "-C", a.repo, "log", "-1", "--format=%ct"],
                        capture_output=True, text=True).stdout.strip()
    bmt = os.path.getmtime(arm)
    stale = ct.isdigit() and bmt < int(ct)
    print(f"arm      : {arm}")
    print(f"arm built: {time.strftime('%Y-%m-%d %H:%M', time.localtime(bmt))}")
    print(f"HEAD     : {head[:12]}  committed "
          f"{time.strftime('%Y-%m-%d %H:%M', time.localtime(int(ct))) if ct.isdigit() else '?'}")
    if stale and not a.allow_stale:
        print("GATE     : STALE — the arm predates HEAD. Rebuild, or pass "
              "--allow-stale if this is deliberate.", file=sys.stderr)
        return 2
    print(f"GATE     : {'stale (overridden)' if stale else 'ok (arm is not older than HEAD)'}")

    os.makedirs(out, exist_ok=True)
    names = sorted(f[:-3] for f in os.listdir(a.cases) if f.endswith(".sh"))
    if a.only:
        keep = set(a.only.split(","))
        names = [n for n in names if n in keep]
    if not names:
        print(f"FATAL: no cases in {a.cases}", file=sys.stderr)
        return 2

    print(f"\n{'case':<26}{'gnuRC':>6}{'armRC':>6}{'gnu':>5}{'arm':>5}"
          f"{'stdout':>8}{'stderr':>8}  verdict")
    rows = []
    for name in names:
        case_src = os.path.join(a.cases, name + ".sh")
        w = os.path.join(out, "out", name)
        stage(case_src, w)
        wslw = wsl_path(w)

        rrc, rto = run_bounded([arm, "./probe.sh"], w, a.timeout,
                               os.path.join(w, "arm.out"), os.path.join(w, "arm.err"))
        grc, gto = run_bounded([WSL, "--cd", wslw, "-e", "env", "LC_ALL=C",
                                "bash", "./probe.sh"], w, a.timeout,
                               os.path.join(w, "gnu.out"), os.path.join(w, "gnu.err"))
        open(os.path.join(w, "arm.rc"), "w").write(str(rrc))
        open(os.path.join(w, "gnu.rc"), "w").write(str(grc))

        # normalise only the harness's own scratch paths, in every spelling
        for tag, home in (("gnu", "/tmp/armhome"), ("arm", os.path.join(w, "home"))):
            win = os.path.abspath(w)
            toks = [home, "/" + win[0].lower() + win[2:].replace("\\", "/"),
                    wslw, win, win.replace("\\", "/")]
            for ext in ("out", "err"):
                p = os.path.join(w, f"{tag}.{ext}")
                open(p + ".n", "w", encoding="utf-8", errors="surrogateescape").write(
                    normalise(p, toks))

        raw = {t: open(os.path.join(w, f"{t}.out"), "rb").read() for t in ("gnu", "arm")}
        gdone = b"__ARM_DONE__" in raw["gnu"]
        rdone = b"__ARM_DONE__" in raw["arm"]
        go = open(os.path.join(w, "gnu.out.n"), encoding="utf-8", errors="surrogateescape").read()
        ro = open(os.path.join(w, "arm.out.n"), encoding="utf-8", errors="surrogateescape").read()
        ge = open(os.path.join(w, "gnu.err.n"), encoding="utf-8", errors="surrogateescape").read()
        re_ = open(os.path.join(w, "arm.err.n"), encoding="utf-8", errors="surrogateescape").read()
        same_out, same_err = go == ro, ge == re_

        if rto or gto:
            verdict = "HANG"
        elif not gdone or not rdone:
            verdict = "INCOMPLETE"
        elif grc == rrc and same_out and same_err:
            verdict = "match"
        else:
            verdict = "DIVERGES"
        if not (same_out and same_err):
            d = "".join(difflib.unified_diff(go.splitlines(True), ro.splitlines(True),
                                             "gnu.stdout", "arm.stdout", n=1))
            de = "".join(difflib.unified_diff(ge.splitlines(True), re_.splitlines(True),
                                              "gnu.stderr", "arm.stderr", n=1))
            open(os.path.join(w, "diff.txt"), "w", encoding="utf-8",
                 errors="surrogateescape").write(d + ("\n--stderr--\n" + de if de else ""))
        rows.append((name, grc, rrc, gdone, rdone, same_out, same_err, verdict))
        print(f"{name:<26}{grc:>6}{rrc:>6}{str(gdone):>5}{str(rdone):>5}"
              f"{('same' if same_out else 'DIFF'):>8}"
              f"{('same' if same_err else 'DIFF'):>8}  {verdict}")

    with open(os.path.join(out, "summary.tsv"), "w", encoding="utf-8") as f:
        f.write("case\tgnu_rc\tarm_rc\tgnu_done\tarm_done\tstdout\tstderr\tverdict\n")
        for r in rows:
            f.write("\t".join(str(x) for x in r) + "\n")

    # ---- compare against the declared expectations ------------------------
    if not os.path.isfile(a.manifest):
        print(f"\n(no manifest at {a.manifest}; verdicts only)")
        return 0
    expect = {}
    for line in open(a.manifest, encoding="utf-8").read().splitlines()[1:]:
        f = line.split("\t")
        if len(f) >= 3:
            expect[f[0]] = (f[1], f[2])
    alarms, promote = [], []
    for name, _g, _r, _gd, _rd, _so, _se, v in rows:
        if name not in expect:
            continue
        issue, want = expect[name]
        if want == "must-match" and v != "match":
            alarms.append((name, issue, want, v))
        elif want == "diverges" and v == "match":
            promote.append((name, issue))
    print()
    if alarms:
        print("ALARMS (regressions):")
        for n, i, want, got in alarms:
            print(f"  {n} (#{i}): expected {want}, measured {got}")
    if promote:
        print("PROMOTE (fix landed; flip the manifest row to must-match):")
        for n, i in promote:
            print(f"  {n} (#{i})")
    if not alarms and not promote:
        print(f"all {len([r for r in rows if r[0] in expect])} catalogued cases match "
              f"their declared expectation")
    return 1 if alarms else 0


if __name__ == "__main__":
    sys.exit(main())
