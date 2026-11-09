#!/usr/bin/env python3
"""rubash#2905 chain runner: bash-completion pytest behaviour suite vs rubash.

Runs the vendored bash-completion pytest+pexpect suite (ecosuite pin 2.18.0)
end-to-end against a built rubash binary, through the pexpect bridge relay
(scripts/ecosuite-bash-completion-relay.py), and writes a failure ledger:
totals plus a histogram of error/failure signatures, so divergences are
filed by mechanism instead of being absorbed into a suite tally.

This is an on-demand eco chain, not a CI job: whether any of this is wired
into CI is the open decision in rubash#3033. The suite itself is not in
this repo; point --suite at a bash-completion checkout (the ecosuite
workspace keeps one at vendor/bash-completion).

Exit status: 0 when pytest executed to completion (pass/fail mix is data
for the ledger, not a harness failure); 2 when the harness itself failed
(no results collected).

Usage:
    run-bash-completion-chain.py --shell /path/to/rubash \
        --suite /path/to/bash-completion [--jobs 4] [--out DIR]
"""

import argparse
import collections
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
RELAY = os.path.join(HERE, "ecosuite-bash-completion-relay.py")


def signature(text):
    """Reduce a failure/error body to its mechanism signature."""
    if not text:
        return "<empty>"
    m = re.search(r"Unexpected output from \"([^\"]+)\"", text)
    if m:
        out = re.search(r'output="([^"\n]*)', text)
        return "fixture-exec: %s -> %s" % (
            os.path.basename(m.group(1)),
            (out.group(1).strip() if out else "?")[:80],
        )
    lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
    for ln in lines:
        if "Error" in ln or "assert" in ln.lower():
            return ln[:120]
    return (lines[-1] if lines else "<empty>")[:120]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--shell", required=True, help="path to the rubash binary")
    ap.add_argument("--suite", required=True, help="bash-completion checkout")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", default=".", help="directory for junit + ledger")
    ap.add_argument("--python", default=sys.executable,
                    help="python with pytest+pexpect installed")
    args = ap.parse_args()

    testdir = os.path.join(args.suite, "test")
    if not os.path.isdir(os.path.join(testdir, "t")):
        sys.stderr.write("suite layout not found under %s\n" % args.suite)
        return 2
    os.makedirs(args.out, exist_ok=True)
    junit = os.path.join(args.out, "bash-completion-junit.xml")
    ledger = os.path.join(args.out, "bash-completion-ledger.txt")

    env = os.environ.copy()
    env["BASH_COMPLETION_TEST_BASH"] = "%s %s %s" % (
        args.python, RELAY, os.path.abspath(args.shell))

    cmd = [args.python, "-m", "pytest", "t", "-q",
           "-n", str(args.jobs), "--junitxml", junit]
    proc = subprocess.run(cmd, cwd=testdir, env=env)
    if not os.path.exists(junit):
        sys.stderr.write("pytest produced no junit output (rc=%d)\n" % proc.returncode)
        return 2

    total = fails = errs = skips = 0
    sigs = collections.Counter()
    named = collections.defaultdict(list)
    for tc in ET.parse(junit).getroot().iter("testcase"):
        total += 1
        for kind, bucket in (("failure", "failures"), ("error", "errors")):
            node = tc.find(kind)
            if node is not None:
                if kind == "failure":
                    fails += 1
                else:
                    errs += 1
                sig = signature(node.text or "")
                sigs[sig] += 1
                if len(named[sig]) < 5:
                    named[sig].append("%s::%s" % (tc.get("classname"), tc.get("name")))
        if tc.find("skipped") is not None:
            skips += 1

    with open(ledger, "w") as fh:
        fh.write("bash-completion chain ledger (shell=%s)\n" % args.shell)
        fh.write("tests=%d failures=%d errors=%d skipped=%d passed=%d\n"
                 % (total, fails, errs, skips, total - fails - errs - skips))
        fh.write("\nsignatures (mechanism -> count, first cases):\n")
        for sig, count in sigs.most_common():
            fh.write("%6d  %s\n" % (count, sig))
            for name in named[sig]:
                fh.write("        e.g. %s\n" % name)
    sys.stdout.write(open(ledger).read())
    return 0


if __name__ == "__main__":
    sys.exit(main())
