#!/usr/bin/env python3
"""check-rustfmt.py -- command-line-length-safe `cargo fmt --check` gate.

`cargo fmt --check` is the correct full-workspace format check, but on
Windows it hands every workspace target root to a single rustfmt
invocation. This workspace has ~1.5k target roots (one per tests/*.rs
stem), which overflows the CreateProcess command-line limit and fails
with os error 206 before any file is checked. This script preserves the
same coverage -- every target src_path of every workspace member, grouped
by edition -- but invokes the installed rustfmt directly in batches whose
command line stays well under the Windows limit.

Discovery uses `cargo metadata --no-deps --format-version 1`; only
packages listed in `workspace_members` are checked (registry/git
dependencies are excluded, matching `cargo fmt`). Each batch runs
`rustfmt --check --edition <edition> <files...>` with cwd at the repo
root so rustfmt discovers rustfmt.toml/.rustfmt.toml exactly as usual.

The gate fails closed: missing cargo or rustfmt, cargo metadata failure,
invalid metadata, or any rustfmt batch returning nonzero all produce a
nonzero exit. There is no fallback to checking only changed files.

Usage:
    python3 scripts/check-rustfmt.py [--repo-root DIR] [--manifest-path PATH]

Exit codes: 0 = all targets formatted, 1 = rustfmt reported diffs/errors,
2 = gate/tooling error.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys

# Windows CreateProcess limits lpCommandLine to 32767 UTF-16 code units
# including the terminating NUL. 12000 is a conservative bound that keeps
# batches far below the limit while still packing ~100+ paths per call.
MAX_CMDLINE_UNITS = 12000


def utf16_units(text):
    return len(text.encode("utf-16-le")) // 2


def cmdline_units(argv):
    """UTF-16 length of the command line Windows would build for argv."""
    return utf16_units(subprocess.list2cmdline([os.fspath(a) for a in argv]))


def find_executable(name):
    return shutil.which(name)


def run_inherit(argv, cwd):
    """Run argv with stdout/stderr inherited; return the CompletedProcess."""
    return subprocess.run([os.fspath(a) for a in argv], cwd=cwd)


def run_capture_stdout(argv, cwd):
    """Run argv capturing stdout (stderr still inherited); return result."""
    return subprocess.run(
        [os.fspath(a) for a in argv], cwd=cwd, stdout=subprocess.PIPE
    )


class GateError(Exception):
    pass


def load_workspace_targets(repo_root, cargo, manifest_path=None):
    """Return {edition: sorted unique target src_paths} for workspace members."""
    argv = [cargo, "metadata", "--no-deps", "--format-version", "1"]
    if manifest_path:
        argv += ["--manifest-path", os.fspath(manifest_path)]
    try:
        proc = run_capture_stdout(argv, cwd=repo_root)
    except OSError as exc:
        raise GateError(f"failed to run cargo metadata: {exc}")
    if proc.returncode != 0:
        raise GateError(
            f"cargo metadata failed with exit code {proc.returncode}"
        )
    try:
        meta = json.loads(proc.stdout.decode("utf-8-sig"))
    except (ValueError, UnicodeDecodeError) as exc:
        raise GateError(f"cargo metadata produced invalid JSON: {exc}")

    packages = {p.get("id"): p for p in meta.get("packages") or []}
    members = set(meta.get("workspace_members") or [])
    member_pkgs = [packages[pid] for pid in members if pid in packages]
    if not member_pkgs:
        raise GateError("cargo metadata reported no workspace member packages")

    by_edition = {}
    for pkg in member_pkgs:
        pkg_edition = pkg.get("edition") or "2015"
        for target in pkg.get("targets") or []:
            src = target.get("src_path")
            if not src:
                continue
            edition = target.get("edition") or pkg_edition
            path = os.path.normpath(os.path.abspath(os.fspath(src)))
            by_edition.setdefault(edition, set()).add(path)
    if not any(by_edition.values()):
        raise GateError("no Rust target sources found in workspace members")
    return {edition: sorted(paths) for edition, paths in by_edition.items()}


def plan_batches(rustfmt, edition, roots):
    """Greedy-pack roots into rustfmt argv batches within the unit bound."""
    prefix = [rustfmt, "--check", "--edition", str(edition)]
    batches = []
    current = []
    for root in roots:
        if cmdline_units(prefix + current + [root]) <= MAX_CMDLINE_UNITS:
            current.append(root)
            continue
        if current:
            batches.append(current)
            current = []
        if cmdline_units(prefix + [root]) > MAX_CMDLINE_UNITS:
            raise GateError(
                "single target path exceeds the "
                f"{MAX_CMDLINE_UNITS} UTF-16 command-line unit bound: {root}"
            )
        current = [root]
    if current:
        batches.append(current)
    return batches


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="cargo fmt --check equivalent that batches rustfmt "
        "invocations to stay under the Windows command-line length limit."
    )
    parser.add_argument(
        "--repo-root",
        default=os.getcwd(),
        help="directory used as cwd for cargo metadata and rustfmt "
        "(default: current directory)",
    )
    parser.add_argument(
        "--manifest-path",
        default=None,
        help="optional Cargo.toml path forwarded to cargo metadata",
    )
    args = parser.parse_args(argv)

    repo_root = os.path.abspath(args.repo_root)
    if not os.path.isdir(repo_root):
        print(f"check-rustfmt: repo root is not a directory: {repo_root}",
              file=sys.stderr)
        return 2

    cargo = find_executable("cargo")
    if cargo is None:
        print("check-rustfmt: cargo not found on PATH; cannot enumerate "
              "workspace targets", file=sys.stderr)
        return 2
    rustfmt = find_executable("rustfmt")
    if rustfmt is None:
        print("check-rustfmt: rustfmt not found on PATH", file=sys.stderr)
        return 2

    try:
        by_edition = load_workspace_targets(repo_root, cargo, args.manifest_path)
    except GateError as exc:
        print(f"check-rustfmt: {exc}", file=sys.stderr)
        return 2

    total = 0
    batches_run = 0
    failed_batches = 0
    for edition in sorted(by_edition):
        roots = by_edition[edition]
        try:
            batches = plan_batches(rustfmt, edition, roots)
        except GateError as exc:
            print(f"check-rustfmt: {exc}", file=sys.stderr)
            return 2
        total += len(roots)
        for batch in batches:
            argv_batch = [rustfmt, "--check", "--edition", str(edition)] + batch
            try:
                proc = run_inherit(argv_batch, cwd=repo_root)
            except OSError as exc:
                print(f"check-rustfmt: failed to run rustfmt: {exc}",
                      file=sys.stderr)
                return 2
            batches_run += 1
            if proc.returncode != 0:
                failed_batches += 1

    if failed_batches:
        print(f"check-rustfmt: rustfmt --check reported diffs or errors in "
              f"{failed_batches} of {batches_run} batch(es)", file=sys.stderr)
        return 1
    editions = ", ".join(f"{e}: {len(by_edition[e])}" for e in sorted(by_edition))
    print(f"check-rustfmt: ok - {total} target(s) formatted "
          f"({editions}) in {batches_run} batch(es)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
