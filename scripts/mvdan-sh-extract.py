#!/usr/bin/env python3
"""mvdan/sh 559-snippet extractor (rubash #2906, pinned).

Reads mvdan/sh's syntax/filetests_test.go at a pinned commit and extracts
the FIRST `in` string of every fileTest() call across the three tables
(fileTests, fileTestsNoPrint, fileTestsKeepComments), in file order,
as one shell snippet per file: tNNNN.sh (0-based), with a trailing newline
appended (matching the 16 fixtures pinned in the rubash repo).

Historical-numbering exception (documented, validated):
  Exactly one call is skipped: fileTests call index 169, variants
  ["{ }", "{}"] — its first string duplicates the immediately preceding
  call's first string ("{ }"). Skipping it reproduces all 16 pinned
  fixture numbers (t0026..t0526) and yields the audit's total of 559
  snippets (560 calls - 1). The skipped call is still emitted as an
  appendix file (tSKIP.sh) so no snippet is lost from the differential;
  the identity of the historical skip cannot be proven from surviving
  evidence (the other candidate is call 167 "{ foo; }") and is recorded
  in the REPORT, not silently assumed.

  Validation gate: the 16 files under the rubash repo's
  tests/fixtures/ecosuite/mvdan-n/ must match the emitted tNNNN.sh
  byte-for-byte. If any differs, the extractor is wrong — do not proceed.

Usage:
  mvdan-sh-extract.py <filetests_test.go> <out-dir> [--check <fixtures-dir>]
  --check verifies the 16 pinned fixtures and exits nonzero on mismatch.

Go string literals handled: "..." with \\n \\t \\r \\\\ \\" \\x41 \\u1234
\\U00012345 and octal \\123 escapes; `...` raw strings verbatim.
"""
import os
import re
import sys

SKIP_CALL = 169  # fileTests call index whose first string duplicates call 168
TABLES = ["fileTests", "fileTestsNoPrint", "fileTestsKeepComments"]


def go_decode(s):
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        i += 1
        e = s[i]
        i += 1
        m = {"a": "\a", "b": "\b", "f": "\f", "n": "\n", "r": "\r",
             "t": "\t", "v": "\v", "\\": "\\", '"': '"', "'": "'", "`": "`"}
        if e in m:
            out.append(m[e])
        elif e == "x":
            out.append(chr(int(s[i:i + 2], 16)))
            i += 2
        elif e == "u":
            out.append(chr(int(s[i:i + 4], 16)))
            i += 4
        elif e == "U":
            out.append(chr(int(s[i:i + 8], 16)))
            i += 8
        elif e.isdigit():
            out.append(chr(int(s[i - 1:i + 2], 8)))
            i += 2
        else:
            out.append("\\" + e)
    return "".join(out)


def scan_str(s, i):
    if s[i] == "`":
        j = s.index("`", i + 1)
        return s[i + 1:j], j + 1
    j = i + 1
    buf = []
    while s[j] != '"':
        if s[j] == "\\":
            buf.append(s[j:j + 2])
            j += 2
        else:
            buf.append(s[j])
            j += 1
    return go_decode("".join(buf)), j + 1


def table_region(src, name):
    m = re.search(r"var %s = \[\]\w+\{" % name, src)
    if not m:
        raise ValueError("table %s not found" % name)
    start = m.end() - 1
    depth = 0
    i = start
    while True:
        c = src[i]
        if c in "\"`":
            _, i = scan_str(src, i)
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return src[start:i + 1]
        i += 1


def calls_with_ins(region):
    out = []
    for m in re.finditer(r"fileTest\(", region):
        i = m.end()
        j = region.index("[", i)
        k = region.index("{", j)
        depth = 0
        p = k
        strs = []
        while True:
            c = region[p]
            if c in "\"`":
                s, p = scan_str(region, p)
                strs.append(s)
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            p += 1
        out.append(strs)
    return out


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    src_path, out_dir = sys.argv[1], sys.argv[2]
    check_dir = None
    if "--check" in sys.argv:
        check_dir = sys.argv[sys.argv.index("--check") + 1]
    src = open(src_path, encoding="utf-8").read()
    os.makedirs(out_dir, exist_ok=True)

    numbered = []  # (number, table, call_index, first_string, n_variants)
    skipped = []
    n = 0
    manifest = []
    for table in TABLES:
        calls = calls_with_ins(table_region(src, table))
        for ci, strs in enumerate(calls):
            first = strs[0]
            if table == "fileTests" and ci == SKIP_CALL:
                skipped.append((table, ci, first, len(strs)))
                continue
            numbered.append((n, table, ci, first, len(strs)))
            n += 1
    for (num, table, ci, first, nvar) in numbered:
        with open(os.path.join(out_dir, "t%04d.sh" % num), "w",
                  encoding="utf-8", newline="\n") as f:
            f.write(first)
            if not first.endswith("\n"):
                f.write("\n")
        manifest.append("%04d\t%s\t%d\t%d\t%s" % (
            num, table, ci, nvar,
            first.encode("unicode_escape").decode()[:120]))
    for (table, ci, first, nvar) in skipped:
        with open(os.path.join(out_dir, "tSKIP.sh"), "w",
                  encoding="utf-8", newline="\n") as f:
            f.write(first)
            if not first.endswith("\n"):
                f.write("\n")
    with open(os.path.join(out_dir, "MANIFEST.tsv"), "w", encoding="utf-8") as f:
        f.write("number\ttable\tcall_index\tn_variants\tfirst_string_escaped\n")
        f.write("\n".join(manifest) + "\n")
    print("extracted %d snippets (+ %d skipped appendix) -> %s"
          % (len(numbered), len(skipped), out_dir))

    if check_dir:
        bad = 0
        for fname in sorted(os.listdir(check_dir)):
            if not fname.startswith("t") or not fname.endswith(".sh"):
                continue
            want = open(os.path.join(check_dir, fname), "rb").read()
            got_path = os.path.join(out_dir, fname)
            if not os.path.exists(got_path):
                print("CHECK FAIL: %s missing from extraction" % fname)
                bad += 1
                continue
            got = open(got_path, "rb").read()
            if got != want:
                print("CHECK FAIL: %s differs" % fname)
                bad += 1
        if bad:
            print("check: %d fixture(s) MISMATCH — extractor is wrong" % bad)
            return 1
        print("check: all pinned fixtures reproduce byte-for-byte")
    return 0


if __name__ == "__main__":
    sys.exit(main())
