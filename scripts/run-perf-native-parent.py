#!/usr/bin/env python3
"""Native-parent perf calibrator (envfix3).

The perf suite's rubash side is timed by an MSYS (Git Bash) parent, whose
fork/exec emulation adds a ~54ms constant to EVERY child spawn (perf10:
cmd.exe pays it identically; GNU's WSL-internal timing pays nothing
comparable). This harness removes the MSYS parent from the timed window:
a native Python process spawns the shell under test directly through
CreateProcess and measures wall clock around the child only.

Usage (from any shell, python >= 3.7):
  python scripts/run-perf-native-parent.py [--runs N] [--probe substr] \
      [--rubash PATH] [--all]

Prints one TSV line per probe: name, median_ms, min_ms, max_ms, runs, rc.
The GNU side is NOT run here; the suite's inner-WSL GNU numbers stay the
comparison baseline (same-session re-run recommended).
"""

import argparse
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "benchmarks"


def meta_field(path: Path, key: str) -> str:
    for line in path.read_text(encoding="utf8", errors="replace").splitlines():
        if not line.startswith("# PERF:"):
            continue
        for token in line.split():
            name, sep, value = token.partition("=")
            if sep and name == key:
                return value.strip('"')
    return ""


def run_probe(shell: str, probe: Path, runs: int) -> str:
    args = meta_field(probe, "args").split()
    cmd = [shell, *args, str(probe)]
    timeout_s = int(meta_field(probe, "timeout") or 60)
    # validation run (warmup) — not timed
    try:
        subprocess.run(
            cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            stdin=subprocess.DEVNULL, timeout=timeout_s, check=False,
        )
    except subprocess.TimeoutExpired:
        return f"{probe.stem}\tTIMEOUT"
    times = []
    rc = 0
    for _ in range(runs):
        t0 = time.perf_counter()
        try:
            proc = subprocess.run(
                cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                stdin=subprocess.DEVNULL, timeout=timeout_s, check=False,
            )
            rc = proc.returncode
        except subprocess.TimeoutExpired:
            return f"{probe.stem}\tTIMEOUT"
        times.append((time.perf_counter() - t0) * 1000.0)
    return (
        f"{probe.stem}\t{statistics.median(times):.1f}\t"
        f"{min(times):.1f}\t{max(times):.1f}\t{runs}\t{rc}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--probe", default="", help="substring filter")
    parser.add_argument("--rubash", default=str(ROOT / "target" / "release" / "rubash.exe"))
    parser.add_argument("--all", action="store_true", help="run every probe file")
    cli = parser.parse_args()

    shell = cli.rubash
    if not Path(shell).exists():
        print(f"FATAL: {shell} not found", file=sys.stderr)
        return 1

    default = ["01-startup-empty", "02-startup-fndef", "10-pathmiss-x100",
               "11-pipeline-yes-head", "13-readloop-gen-x2000", "15-expansion-x5000"]
    names = None
    if cli.all:
        names = sorted(p.stem for p in BENCH.glob("[0-9][0-9]-*.sh"))
    elif cli.probe:
        names = sorted(p.stem for p in BENCH.glob(f"*{cli.probe}*.sh"))
    else:
        names = default
    print("# native-parent (CreateProcess) timing; no MSYS parent in the window")
    print(f"# shell {shell}")
    print("probe\tmedian_ms\tmin_ms\tmax_ms\truns\trc")
    for stem in names:
        probe = BENCH / f"{stem}.sh"
        if not probe.exists():
            continue
        print(run_probe(shell, probe, cli.runs), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
