#!/usr/bin/env python3
r"""Blind-consumer lint for the carrier/marker protocol (rubash#2885 E7).

The #2879 sweep (classes (a)/(f) — 64% of its findings) root-caused to
consumers hand-rolling textual surgery on marker-carrying text: a bare
`.replace(CONST, ...)` cannot see the U+E400 literal-char escape pair
that makes a user-typed marker char *data*, so it folds data into
protocol (or strips protocol out of data). The sanctioned helpers in
`src/executor/markers.rs` are pair-aware. This lint is the enforcement
gate that keeps new blind sites from being written:

  L1  No raw PUA-zone marker literals in code position: a char or
      string literal spelling a codepoint in the registry zone
      (U+E000..=U+E4FF, incl. the raw-byte payload range and the
      byte-char range) outside `src/executor/markers.rs`. Use the
      named registry const.
  L2  Same for the C0 carrier codepoints that have registry consts
      (U+0000..=U+0003, U+0005, U+0010..=U+001F): `'\u{11}'` in code
      position is CTLESC — write `markers::CTLESC`.
  L3  No textual needle operations keyed on a registry const
      (`.replace(`, `.contains(`, `.trim_start_matches(`,
      `.trim_end_matches(`, `.trim_matches(`, `.cow_replace(`) at an
      unreviewed site. Reviewed
      sites are allowlisted per (file, method, const) with the
      invariant that makes the bare operation sound there.

Scope note: only the `\u{...}` spelling is linted. The `\x..`
byte spelling is byte-domain data (terminal control bytes, encoding
internals) and out of scope — every marker literal the sweep found in
word/storage text uses the `\u{...}` spelling.

Data uses (readline key tables, ANSI-C decode tables, expected-value
literals in tests) are allowlisted, not converted: the registry const
would not make a data table clearer. Every allowlist entry carries a
justification; a new hit fails with instructions, and a stale entry
(one that no longer matches anything) fails too, so the list cannot
rot. This is a code-hygiene gate for this repository only — it is not
a product mechanism.

Usage: python3 scripts/check-marker-literals.py [--root DIR]
Exit: 0 clean, 1 violations, 2 the lint itself could not run.
"""
import re
import sys
from pathlib import Path

REGISTRY = Path("src/executor/markers.rs")
ALLOWLIST = Path("scripts/marker-lint-allowlist.txt")
NEEDLE_METHODS = (
    "replace",
    "contains",
    "trim_start_matches",
    "trim_end_matches",
    "trim_matches",
    "cow_replace",
)


def parse_registry(text):
    """Registry const names + the marker codepoint sets, from markers.rs."""
    names = set()
    c0 = set()
    pua = set()
    for m in re.finditer(r"pub(?:\(crate\))?\s+const\s+([A-Z][A-Z0-9_]*)\s*:", text):
        names.add(m.group(1))
    for m in re.finditer(r"const\s+[A-Z][A-Z0-9_]*\s*:\s*char\s*=\s*'(\\(?:u\{([0-9A-Fa-f]+)\}|x([0-9A-Fa-f]+)|0))'", text):
        cp = int(m.group(2) or m.group(3) or "0", 16)
        (c0 if cp < 0x20 else pua).add(cp)
    for m in re.finditer(r"const\s+[A-Z][A-Z0-9_]*\s*:\s*&str\s*=\s*\"(\\u\{([0-9A-Fa-f]+)\})?", text):
        if m.group(2):
            cp = int(m.group(2), 16)
            (c0 if cp < 0x20 else pua).add(cp)
    # The raw-byte payload range (RAW_BYTE_MARKER_FIRST..=LAST) and the
    # byte-char range (BYTE_CHAR_BASE..+0x100) are zone ranges, not
    # single consts: any literal inside them is protocol text.
    pua |= set(range(0xE000, 0xE200))
    return names, c0, pua


def strip_comments(text):
    """Remove // and /* */ comments, string-aware enough for this tree.

    A `//` starts a comment only when the number of unescaped double
    quotes before it on the line is even (not inside a string). Block
    comments are tracked across lines outside strings. String contents
    are KEPT: a marker literal inside a string in code position is a
    needle/spelling use and in scope; data strings are allowlisted.
    """
    out = []
    in_block = False
    for line in text.splitlines():
        buf = []
        i = 0
        in_str = False
        while i < len(line):
            two = line[i : i + 2]
            if in_block:
                if two == "*/":
                    in_block = False
                    i += 2
                else:
                    i += 1
                continue
            if in_str:
                buf.append(line[i])
                if line[i] == "\\" and i + 1 < len(line):
                    buf.append(line[i + 1])
                    i += 2
                    continue
                if line[i] == '"':
                    in_str = False
                i += 1
                continue
            if two == "/*":
                in_block = True
                i += 2
                continue
            if two == "//":
                break
            if line[i] == '"':
                in_str = True
            buf.append(line[i])
            i += 1
        out.append("".join(buf))
    return out


def split_arguments(text, start):
    """Split a call's argument list (text starts just past '(') into
    top-level arguments, skipping string/char literals and nested
    (), [], {}."""
    args = []
    depth = 0
    i = start
    arg_start = start
    while i < len(text):
        ch = text[i]
        if ch == '"' or ch == "'":
            j = i + 1
            closed = False
            while j < len(text) and j - i <= (4096 if ch == '"' else 8):
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == ch:
                    closed = True
                    break
                j += 1
            if closed:
                i = j + 1
                continue
            i += 1
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            if depth == 0:
                args.append(text[arg_start:i])
                return args
            depth -= 1
        elif ch == "," and depth == 0:
            args.append(text[arg_start:i])
            arg_start = i + 1
        i += 1
    args.append(text[arg_start:])
    return args

class Allowlist:
    def __init__(self, path):
        self.entries = []  # (relpath, rule, matcher, justification, raw)
        self.errors = []
        if not path.exists():
            self.errors.append(f"allowlist missing: {path}")
            return
        for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = [p.strip() for p in line.split("|")]
            if len(parts) != 4:
                self.errors.append(f"{path}:{lineno}: want 'path | RULE | matcher | justification'")
                continue
            relpath, rule, matcher, justification = parts
            if rule not in ("L1", "L2", "L3"):
                self.errors.append(f"{path}:{lineno}: unknown rule {rule!r}")
                continue
            if len(justification) < 12:
                self.errors.append(f"{path}:{lineno}: justification too short to review")
                continue
            self.entries.append((relpath, rule, matcher, justification, raw))

    def allows_literal(self, relpath, rule, line_text):
        # IGNORECASE: hex spellings in source vary (`\u{1f}` vs `\u{1F}`);
        # the matcher is anchored to the `\u{...}` shape, so case folding
        # cannot over-match prose.
        return [
            e
            for e in self.entries
            if e[0] == relpath
            and e[1] == rule
            and re.search(e[2], line_text, re.IGNORECASE)
        ]

    def allows_needle(self, relpath, method, const_name):
        want = f"{method}:{const_name}"
        return [e for e in self.entries if e[0] == relpath and e[1] == "L3" and e[2] == want]


LITERAL_RE = re.compile(r"\\u\{([0-9A-Fa-f]{1,6})\}")


def main():
    root = Path(__file__).resolve().parent.parent
    if "--root" in sys.argv:
        root = Path(sys.argv[sys.argv.index("--root") + 1]).resolve()
    registry_path = root / REGISTRY
    if not registry_path.exists():
        print(f"marker-lint: registry not found at {registry_path}", file=sys.stderr)
        return 2
    names, c0_set, pua_set = parse_registry(registry_path.read_text(encoding="utf-8"))
    if len(names) < 40 or len(c0_set) < 10 or not pua_set:
        print("marker-lint: registry parse looks empty — refusing to run blind", file=sys.stderr)
        return 2
    allow = Allowlist(root / ALLOWLIST)
    if allow.errors:
        for e in allow.errors:
            print(f"marker-lint: {e}", file=sys.stderr)
        return 2

    used = set()  # ids of allowlist entries that matched something
    violations = []
    # Method calls (.replace() and free-function calls
    # (markers::cow_replace()): both shapes perform the same surgery.
    needle_re = re.compile(r"(?:\.|::)(" + "|".join(NEEDLE_METHODS) + r")\s*\(")
    name_re = re.compile(r"\b(" + "|".join(sorted(names, key=len, reverse=True)) + r")\b")

    src_root = root / "src"
    for path in sorted(src_root.rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        if rel == REGISTRY.as_posix():
            continue  # the registry defines the literals; it is the one exempt file
        lines = strip_comments(path.read_text(encoding="utf-8", errors="replace"))
        for idx, line in enumerate(lines):
            lineno = idx + 1
            # L1 / L2: marker literals in code position.
            for m in LITERAL_RE.finditer(line):
                cp = int(m.group(1), 16)
                rule = "L1" if cp in pua_set else "L2" if cp in c0_set else None
                if rule is None:
                    continue
                hits = allow.allows_literal(rel, rule, line)
                if hits:
                    used.update(id(e) for e in hits)
                else:
                    violations.append(
                        f"{rel}:{lineno}: {rule} raw marker literal U+{cp:04X} in code position — "
                        f"use the registry const from src/executor/markers.rs, or allowlist a data use"
                    )
            # L3: needle operations keyed on a registry const. Window:
            # the call line plus up to 4 following lines (multi-line
            # argument lists), so a const on a continuation line counts.
            for m in needle_re.finditer(line):
                # Only the NEEDLE counts: the first argument of the
                # call (for .replace, the pattern — a const in the
                # replacement position is an encoder, the opposite
                # direction). A const in receiver position — a range
                # bound in `(FIRST..=LAST).contains`, a sentinel string
                # in `SENTINEL.contains(c)` — is a membership test,
                # not surgery on marker-carrying text. The scan
                # continues onto following lines for multi-line
                # argument lists.
                joined = line[m.end() :] + "\n" + "\n".join(lines[idx + 1 : idx + 6])
                args = split_arguments(joined, 0)
                # cow_replace(s, from, to): the needle is the second
                # argument; for every other needle method it is the
                # first (and usually only) one.
                needle_idx = 1 if m.group(1) == "cow_replace" else 0
                window = args[needle_idx] if len(args) > needle_idx else ""
                for nm in name_re.finditer(window):
                    const_name = nm.group(1)
                    hits = allow.allows_needle(rel, m.group(1), const_name)
                    if hits:
                        used.update(id(e) for e in hits)
                    else:
                        violations.append(
                            f"{rel}:{lineno}: L3 .{m.group(1)}( keyed on registry const {const_name} "
                            f"at an unreviewed site — route it through a pair-aware helper in "
                            f"markers.rs, or allowlist it with the invariant that makes it sound"
                        )
                        break  # one report per call site is enough

    stale = [e for e in allow.entries if id(e) not in used]
    for e in stale:
        violations.append(
            f"allowlist: stale entry matches nothing (remove or fix it): {e[4].strip()}"
        )

    if violations:
        print(f"marker-lint: {len(violations)} violation(s)")
        for v in violations:
            print(f"  {v}")
        return 1
    print(
        f"marker-lint: OK ({len(names)} registry consts, "
        f"{len(allow.entries)} allowlisted sites)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
