#!/usr/bin/env python3
"""Compare release binaries and GNU Bash using validated interleaved samples."""

import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--baseline-ref", required=True)
    parser.add_argument("--candidate-ref", required=True)
    parser.add_argument("--runs", type=int, default=9)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("scripts", nargs="+", type=Path)
    args = parser.parse_args()
    if args.runs < 7 or args.warmup < 2 or args.timeout <= 0:
        parser.error("require runs >= 7, warmup >= 2, timeout > 0")
    shells = {
        "baseline": str(Path(args.baseline).resolve(strict=True)),
        "candidate": str(Path(args.candidate).resolve(strict=True)),
        "reference": str(Path(args.reference).resolve(strict=True)),
    }
    environment = dict(os.environ, LC_ALL="C")
    for name in ("BASH_ENV", "ENV", "SHELLOPTS", "BASHOPTS"):
        environment.pop(name, None)
    report = {
        "platform": platform.platform(),
        "python": platform.python_version(),
        "profile": "release (opt-level=3, lto=true, codegen-units=1)",
        "baseline_ref": args.baseline_ref,
        "candidate_ref": args.candidate_ref,
        "warmup": args.warmup,
        "runs": args.runs,
        "method": "rotating interleaved order, native Linux parent, perf_counter_ns",
        "shells": {},
        "workloads": [],
    }
    for name, executable in shells.items():
        version = subprocess.run(
            [executable, "--version"], capture_output=True, timeout=args.timeout,
            env=environment, check=True,
        )
        report["shells"][name] = {
            "path": executable,
            "sha256": hashlib.sha256(Path(executable).read_bytes()).hexdigest(),
            "version": version.stdout.decode(errors="replace").splitlines()[0],
        }
    for script in args.scripts:
        script = script.resolve(strict=True)
        samples = {name: [] for name in shells}
        expected = None
        names = list(shells)
        for iteration in range(args.warmup + args.runs):
            offset = iteration % len(names)
            for name in names[offset:] + names[:offset]:
                started = time.perf_counter_ns()
                result = subprocess.run(
                    [shells[name], str(script)], capture_output=True,
                    stdin=subprocess.DEVNULL, timeout=args.timeout, env=environment,
                )
                elapsed = (time.perf_counter_ns() - started) / 1_000_000
                actual = (result.returncode, result.stdout, result.stderr)
                if expected is None:
                    expected = actual
                if result.returncode != 0 or actual != expected:
                    raise RuntimeError(f"{script.name}: {name} output/exit mismatch: {actual!r}")
                if iteration >= args.warmup:
                    samples[name].append(elapsed)
        measurements = {
            name: {
                "median_ms": statistics.median(values),
                "min_ms": min(values), "max_ms": max(values), "samples_ms": values,
            }
            for name, values in samples.items()
        }
        candidate = measurements["candidate"]["median_ms"]
        report["workloads"].append({
            "script": script.name,
            "script_sha256": hashlib.sha256(script.read_bytes()).hexdigest(),
            "output_equal": True,
            "stdout": expected[1].decode(errors="replace"),
            "measurements": measurements,
            "candidate_over_baseline": candidate / measurements["baseline"]["median_ms"],
            "candidate_over_reference": candidate / measurements["reference"]["median_ms"],
        })
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
