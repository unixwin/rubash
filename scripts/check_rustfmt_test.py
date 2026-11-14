#!/usr/bin/env python3
"""Unit tests for check-rustfmt.py -- run with:

    python scripts/check_rustfmt_test.py        (from repo root or scripts/)
    python -m unittest check_rustfmt_test -v    (from scripts/)

The script under test is dash-named, so it is loaded by path. All tool
lookups and subprocess calls are mocked; no real cargo/rustfmt runs.
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "check_rustfmt", os.path.join(HERE, "check-rustfmt.py")
)
check_rustfmt = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_rustfmt)

TOOLS = {"cargo": "C:/rust/cargo.exe", "rustfmt": "C:/rust/rustfmt.exe"}


def norm(path):
    """Mirror the script's src_path normalization."""
    return os.path.normpath(os.path.abspath(os.fspath(path)))


def make_pkg(pkg_id, edition, src_paths, target_editions=None):
    targets = []
    for i, src in enumerate(src_paths):
        target = {"src_path": src, "kind": ["lib" if i == 0 else "test"]}
        ed = (target_editions or {}).get(src, edition)
        if ed is not None:
            target["edition"] = ed
        targets.append(target)
    return {"id": pkg_id, "name": pkg_id, "edition": edition,
            "targets": targets}


def make_meta(packages, members):
    return {"packages": packages, "workspace_members": members,
            "workspace_root": "D:/repo/proj"}


class GateTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.repo_root = self._tmp.name

    def tearDown(self):
        self._tmp.cleanup()

    def run_gate(self, meta=None, tools=None, capture_rc=0,
                 inherit_rc=0, extra_args=()):
        tools = TOOLS if tools is None else tools
        capture_calls = []
        inherit_calls = []
        stdout = json.dumps(meta).encode() if meta is not None else b""

        def fake_capture(argv, cwd):
            capture_calls.append((list(argv), cwd))
            return subprocess.CompletedProcess(argv, capture_rc,
                                               stdout=stdout)

        def fake_inherit(argv, cwd):
            inherit_calls.append((list(argv), cwd))
            return subprocess.CompletedProcess(argv, inherit_rc)

        with mock.patch.object(check_rustfmt, "find_executable",
                               side_effect=lambda name: tools.get(name)), \
             mock.patch.object(check_rustfmt, "run_capture_stdout",
                               new=fake_capture), \
             mock.patch.object(check_rustfmt, "run_inherit",
                               new=fake_inherit):
            rc = check_rustfmt.main(["--repo-root", self.repo_root]
                                    + list(extra_args))
        return rc, capture_calls, inherit_calls

    @staticmethod
    def rustfmt_files(call):
        argv, _cwd = call
        assert argv[0] == TOOLS["rustfmt"]
        assert argv[1] == "--check"
        assert argv[2] == "--edition"
        return argv[3], argv[4:]

    def test_all_roots_covered_across_editions_members_only(self):
        roots_a = [r"D:\repo\proj\src\lib.rs",
                   r"D:\repo\proj\tests\a.rs",
                   r"D:\repo\proj\tests\b.rs"]
        roots_b = [r"D:\repo\proj\crates\sub\src\lib.rs",
                   r"D:\repo\proj\crates\sub\tests\c.rs"]
        dep_root = r"D:\repo\proj\..\registry\dep-1.0\src\lib.rs"
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg-a@1.0", "2021", roots_a),
             make_pkg("path+file:///D:/repo/proj/crates/sub#pkg-b@2.0",
                      "2018", roots_b,
                      target_editions={roots_b[1]: "2021"}),
             make_pkg("registry+https://x#dep@1.0", "2021", [dep_root])],
            ["path+file:///D:/repo/proj#pkg-a@1.0",
             "path+file:///D:/repo/proj/crates/sub#pkg-b@2.0"])

        rc, capture_calls, inherit_calls = self.run_gate(meta=meta)

        self.assertEqual(rc, 0)
        # cargo metadata ran once, cwd at repo root, --no-deps JSON format.
        self.assertEqual(len(capture_calls), 1)
        meta_argv, meta_cwd = capture_calls[0]
        self.assertEqual(meta_cwd, self.repo_root)
        self.assertEqual(meta_argv[:2], [TOOLS["cargo"], "metadata"])
        self.assertIn("--no-deps", meta_argv)
        self.assertIn("--format-version", meta_argv)

        seen = {}
        for call in inherit_calls:
            edition, files = self.rustfmt_files(call)
            for f in files:
                self.assertNotIn(f, seen, "file checked twice")
                seen[f] = edition
            _argv, cwd = call
            self.assertEqual(cwd, self.repo_root)
            self.assertLessEqual(
                check_rustfmt.cmdline_units(_argv),
                check_rustfmt.MAX_CMDLINE_UNITS)

        expected = {norm(r): e
                    for r, e in [(r, "2021") for r in roots_a]
                    + [(roots_b[0], "2018"), (roots_b[1], "2021")]}
        got = {norm(k): v for k, v in seen.items()}
        self.assertEqual(got, expected)
        self.assertNotIn(norm(dep_root), got)

    def test_batches_obey_bound_with_spaces(self):
        roots = [
            "D:\\repo\\my proj\\tests\\" + "t%03d_" % i + "x" * 80 + ".rs"
            for i in range(150)
        ]
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021", roots)],
            ["path+file:///D:/repo/proj#pkg@1.0"])

        rc, _cap, inherit_calls = self.run_gate(meta=meta)

        self.assertEqual(rc, 0)
        self.assertGreater(len(inherit_calls), 1,
                           "expected the roots to require multiple batches")
        all_files = []
        for call in inherit_calls:
            argv, _cwd = call
            self.assertIsInstance(argv, list)
            self.assertLessEqual(
                check_rustfmt.cmdline_units(argv),
                check_rustfmt.MAX_CMDLINE_UNITS)
            _edition, files = self.rustfmt_files(call)
            all_files.extend(files)
        self.assertEqual(sorted(norm(f) for f in all_files),
                         sorted(norm(r) for r in roots))
        self.assertEqual(len(all_files), len(set(all_files)))

    def test_duplicate_src_path_checked_once(self):
        shared = r"D:\repo\proj\src\lib.rs"
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021",
                      [shared, shared])],
            ["path+file:///D:/repo/proj#pkg@1.0"])
        rc, _cap, inherit_calls = self.run_gate(meta=meta)
        self.assertEqual(rc, 0)
        files = [f for call in inherit_calls
                 for f in self.rustfmt_files(call)[1]]
        self.assertEqual(files, [norm(shared)])

    def test_manifest_path_forwarded(self):
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021",
                      [r"D:\repo\proj\src\lib.rs"])],
            ["path+file:///D:/repo/proj#pkg@1.0"])
        rc, capture_calls, _inh = self.run_gate(
            meta=meta, extra_args=["--manifest-path", "sub/Cargo.toml"])
        self.assertEqual(rc, 0)
        self.assertIn("sub/Cargo.toml", capture_calls[0][0])

    def test_metadata_failure_fails_closed(self):
        rc, _cap, inherit_calls = self.run_gate(capture_rc=1)
        self.assertNotEqual(rc, 0)
        self.assertEqual(inherit_calls, [])

    def test_invalid_metadata_json_fails_closed(self):
        tools = TOOLS
        with mock.patch.object(check_rustfmt, "find_executable",
                               side_effect=lambda n: tools.get(n)), \
             mock.patch.object(
                 check_rustfmt, "run_capture_stdout",
                 new=lambda argv, cwd: subprocess.CompletedProcess(
                     argv, 0, stdout=b"not json")):
            rc = check_rustfmt.main(["--repo-root", self.repo_root])
        self.assertNotEqual(rc, 0)

    def test_missing_cargo_fails_closed(self):
        rc, cap, inh = self.run_gate(tools={"rustfmt": TOOLS["rustfmt"]})
        self.assertNotEqual(rc, 0)
        self.assertEqual(cap, [])
        self.assertEqual(inh, [])

    def test_missing_rustfmt_fails_closed(self):
        rc, cap, inh = self.run_gate(tools={"cargo": TOOLS["cargo"]})
        self.assertNotEqual(rc, 0)
        self.assertEqual(cap, [])
        self.assertEqual(inh, [])

    def test_empty_workspace_members_fails_closed(self):
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021",
                      [r"D:\repo\proj\src\lib.rs"])],
            [])
        rc, _cap, inh = self.run_gate(meta=meta)
        self.assertNotEqual(rc, 0)
        self.assertEqual(inh, [])

    def test_rustfmt_nonzero_fails_gate(self):
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021",
                      [r"D:\repo\proj\src\lib.rs"])],
            ["path+file:///D:/repo/proj#pkg@1.0"])
        rc, _cap, inh = self.run_gate(meta=meta, inherit_rc=1)
        self.assertEqual(rc, 1)
        self.assertEqual(len(inh), 1)

    def test_single_overlong_path_rejected(self):
        huge = "D:\\" + "a" * (check_rustfmt.MAX_CMDLINE_UNITS + 100) + ".rs"
        meta = make_meta(
            [make_pkg("path+file:///D:/repo/proj#pkg@1.0", "2021",
                      [huge])],
            ["path+file:///D:/repo/proj#pkg@1.0"])
        rc, _cap, inh = self.run_gate(meta=meta)
        self.assertNotEqual(rc, 0)
        self.assertEqual(inh, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
