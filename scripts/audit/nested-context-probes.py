#!/usr/bin/env python3
"""#713 nested-context inheritance probes: subshell `( )`, `$( )`, pipeline
stages and background jobs against a genuine GNU bash.

Each case runs in its own file (`bash <file>` / `rubash <file>`) with stdout,
stderr and exit status compared separately. Do NOT run the GNU leg as
`wsl bash -lc "bash -c '...'"`: WSL re-joins the argument list, the single
quotes vanish, and `;` is then interpreted by the outer shell (it silently
fabricates `$?`/`$x` divergences).

Override the binaries with RUBASH_BIN / GNU_CMD:
    RUBASH_BIN=<path>/rubash.exe python scripts/audit/nested-context-probes.py
"""
import subprocess, os, pathlib

RB = os.environ.get(
    "RUBASH_BIN", r"D:/repo/rubash-wt-392-lookup/target/debug/rubash.exe"
)
# The GNU leg must read the case from a FILE: `wsl bash -lc "bash -c ..."`
# re-joins the argument list and strips quotes (see the harness notes above).
GNU_PREFIX = os.environ.get("GNU_CMD", "wsl bash").split()
here = pathlib.Path(__file__).resolve().parent
cases = [
 ("fd9-in-subshell",        "exec 9<&0; ( IFS= read -r -u9 x </dev/null; echo rc=$? )"),
 ("fd3-in-subshell",        "exec 3>&1; ( echo via3 >&3 )"),
 ("stdout-redirect-inherit","( echo hidden >/dev/null ); echo visible"),
 ("trap-visible-in-subshell","trap 'echo T' USR1; ( trap -p USR1 )"),
 ("exit-trap-in-subshell",  "trap 'echo EXIT' EXIT; ( : ); echo done"),
 ("err-plain-subshell",     "trap 'echo E' ERR; ( false ); echo after"),
 ("err-with-E",             "set -E; trap 'echo E' ERR; ( false ); echo after"),
 ("return-in-funcsub",      "trap 'echo R' RETURN; f() ( : ); f; echo after"),
 ("ignored-signal-inherit", "( trap '' INT; trap -p INT )"),
 ("subshell-exit-code",     "( exit 7 ); echo $?"),
 ("comsub-exit-code",       "x=$(exit 7); echo $?"),
 ("subshell-outer-status",  "( exit 5 ); :; echo $?"),
 ("pipe-status-no-pipefail","false | true; echo $?"),
 ("pipe-status-pipefail",   "set -o pipefail; false | true; echo $?"),
 ("bg-wait-status",         "( exit 9 ) & wait $!; echo $?"),
 ("bg-group-wait-status",   "{ sleep 0; exit 3; } & wait; echo $?"),
 ("subshell-stderr-route",  "( echo E >&2 ) 2>/dev/null; echo rc=$?"),
 ("comsub-stderr-passthru", "x=$(echo E >&2); echo \"got=[$x]\""),
 ("closed-stderr-subshell", "( echo E >&2 ) 2>&-; echo rc=$?"),
 ("pipe-stage-stderr",      "{ echo E >&2; } | cat; echo rc=$?"),
 ("stderr-inside-comsub",   "x=$( { echo E >&2; } 2>&1 ); echo \"got=[$x]\""),
 ("subshell-var-isolation", "x=1; ( x=2 ); echo $x"),
 ("comsub-var-isolation",   "x=1; x=$(echo 2); echo $x"),
 ("bg-var-isolation",       "x=1; x=2 & wait; echo $x"),
]
def _run(argv):
    p = subprocess.run(argv, capture_output=True)
    return (
        p.stdout.decode(errors="replace"),
        p.stderr.decode(errors="replace"),
        p.returncode,
    )


def _case_file(script):
    f = here / "case.sh"
    f.write_text(script.replace("\r\n", "\n"), encoding="utf-8")
    return f


def _as_guest_path(path):
    """Translate for a WSL guest; a native bash just gets the path back.

    Normalize to forward slashes first: WSL's command-line handling eats
    backslashes, so `wslpath` would receive `D:repodirfile` with no
    separators and echo it back unchanged.
    """
    win = str(path).replace("\\", "/")
    out = subprocess.run(["wsl", "wslpath", "-a", win], capture_output=True)
    return out.stdout.decode().strip() or win


def run_rb(script):
    # File form for both legs: `-c` binds a different stdin transport.
    return _run([RB, str(_case_file(script))])


def run_gnu(script):
    f = _case_file(script)
    if GNU_PREFIX[:1] == ["wsl"]:
        return _run(GNU_PREFIX + [_as_guest_path(f)])
    return _run(GNU_PREFIX + [str(f)])


if __name__ == "__main__":
    bad = 0
    for label, script in cases:
        rb, gn = run_rb(script), run_gnu(script)
        if rb != gn:
            bad += 1
            print(
                f"### DIVERGE {label}\n  script   : {script}\n"
                f"  rubash   : out={rb[0]!r} err={rb[1]!r} rc={rb[2]}\n"
                f"  GNU bash : out={gn[0]!r} err={gn[1]!r} rc={gn[2]}"
            )
    print(f"\ncases={len(cases)} divergences={bad}")
    raise SystemExit(1 if bad else 0)
