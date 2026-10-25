#!/usr/bin/env python3
"""API ledger tripwire (rubash#595, M-0 of the 1.5.0 Session API RFC).

The API ledger (docs/api-ledger.md) freezes the engine surface that
niubash actually consumes. This script is the fast, source-level half
of the pairing canary: it parses the ledger's machine-checkable symbol
block and verifies that every recorded symbol still exists in this
tree. A renamed or deleted symbol fails here, in the engine's own CI,
instead of surfacing in the consumer's next build.

The authoritative half of the canary is the pairing CI job
(.github/workflows/niu-canary.yml), which compiles niubash against
this tree. This script is deliberately a tripwire, not a type checker:
it proves presence by declaration patterns, nothing more.

Usage:
    python3 scripts/check-api-ledger.py [--repo PATH] [--ledger PATH]
    python3 scripts/check-api-ledger.py --self-test

Exit status: 0 when every ledger symbol is present, 1 otherwise.
Standard library only.
"""

import argparse
import re
import sys
import tempfile
from pathlib import Path

BLOCK_HEADER = "```api-ledger"

# Declaration patterns that prove a `pub` item exists under a name.
PUB_DECL_PATTERNS = (
    r"pub\s+(?:async\s+)?fn\s+{name}\b",
    r"pub\s+struct\s+{name}\b",
    r"pub\s+enum\s+{name}\b",
    r"pub\s+trait\s+{name}\b",
    r"pub\s+const\s+{name}\b",
    r"pub\s+static\s+{name}\b",
    r"pub\s+type\s+{name}\b",
    r"pub\s+use\s+[^;]*\b{name}\b",
    r"pub\s*\([^)]*\)\s+use\s+[^;]*\b{name}\b",
)


def parse_ledger(text):
    """Extract (kind, path) entries from the ledger's fenced block."""
    entries = []
    in_block = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == BLOCK_HEADER:
            in_block = True
            continue
        if in_block and stripped == "```":
            break
        if in_block and stripped and not stripped.startswith("#"):
            parts = stripped.split()
            if len(parts) == 2 and parts[0] in ("item", "variant", "method"):
                entries.append((parts[0], parts[1]))
    return entries


def module_files(src_root, module_segments):
    """Files that may declare items of a module path (best effort)."""
    files = []
    if not module_segments:
        lib = src_root / "lib.rs"
        if lib.exists():
            files.append(lib)
        return files
    base = src_root.joinpath(*module_segments)
    for candidate in (Path(str(base) + ".rs"), base / "mod.rs"):
        if candidate.exists():
            files.append(candidate)
    if base.is_dir():
        files.extend(sorted(base.rglob("*.rs")))
    return files


def read_all(paths):
    chunks = []
    for path in paths:
        try:
            chunks.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            pass
    return "\n".join(chunks)


def item_exists(src_root, segments):
    """segments: full path after `rubash::`, last segment is the item."""
    name = segments[-1]
    module_segments = segments[:-1]
    files = module_files(src_root, module_segments)
    if module_segments:
        files.append(src_root / "lib.rs")
    text = read_all(files)
    if not text:
        return False
    return any(
        re.search(pattern.format(name=re.escape(name)), text)
        for pattern in PUB_DECL_PATTERNS
    )


def variant_exists(src_root, segments):
    """segments: [..., EnumName, VariantName] after `rubash::`.

    The variant is searched only in files that declare the enum, so a
    root re-export (e.g. `rubash::TokenKind`, defined under `lexer/`)
    resolves to the defining file rather than to lib.rs.
    """
    variant = segments[-1]
    enum_name = segments[-2]
    if not item_exists(src_root, segments[:-1]):
        return False
    decl = re.compile(r"pub\s+enum\s+" + re.escape(enum_name) + r"\b")
    # A variant appears as a bare identifier at a line start (possibly
    # indented), followed by `,`, `(`, `{`, or `=`.
    pattern = r"(?m)^\s*" + re.escape(variant) + r"\s*(?=[,({=])"
    for path in sorted(src_root.rglob("*.rs")):
        text = read_all([path])
        if decl.search(text) and re.search(pattern, text):
            return True
    return False


def method_exists(src_root, segments):
    """segments: [..., TypeName, method_name] after `rubash::`.

    Methods are searched across the whole src/ tree: impl blocks for a
    type are spread over many files in this codebase, and the tripwire
    only needs to know the `pub fn` still exists somewhere.
    """
    name = segments[-1]
    text = read_all(sorted(src_root.rglob("*.rs")))
    pattern = r"pub\s+(?:async\s+)?fn\s+{name}\s*(?:<[^>]*>)?\s*\(".format(
        name=re.escape(name)
    )
    return re.search(pattern, text) is not None


def check(repo_root, ledger_path, verbose=False):
    src_root = repo_root / "src"
    entries = parse_ledger(ledger_path.read_text(encoding="utf-8"))
    if not entries:
        print("check-api-ledger: no symbol block found in ledger", file=sys.stderr)
        return 1
    missing = []
    for kind, path in entries:
        if not path.startswith("rubash::"):
            missing.append((kind, path, "path does not start with rubash::"))
            continue
        segments = path[len("rubash::"):].split("::")
        if kind == "item":
            ok = item_exists(src_root, segments)
        elif kind == "variant":
            ok = variant_exists(src_root, segments)
        else:
            ok = method_exists(src_root, segments)
        if verbose:
            print(f"  {'ok ' if ok else 'MISS'} {kind} {path}")
        if not ok:
            missing.append((kind, path, "not found in tree"))
    if missing:
        print(f"check-api-ledger: {len(missing)} ledger symbol(s) missing:")
        for kind, path, why in missing:
            print(f"  MISSING {kind} {path} ({why})")
        return 1
    print(f"check-api-ledger: all {len(entries)} ledger symbols present")
    return 0


def self_test():
    """The checker must be able to fail: a bogus symbol has to be caught."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text(
            "pub mod executor;\npub use executor::Executor;\n", encoding="utf-8"
        )
        (root / "src" / "executor.rs").write_text(
            "pub struct Executor;\nimpl Executor {\n"
            "    pub fn set_env(&mut self) {}\n}\n",
            encoding="utf-8",
        )
        ledger = root / "ledger.md"
        ledger.write_text(
            "# t\n\n```api-ledger\n"
            "item rubash::executor::Executor\n"
            "method rubash::executor::Executor::set_env\n"
            "item rubash::executor::NoSuchSymbol\n"
            "```\n",
            encoding="utf-8",
        )
        rc = check(root, ledger)
        if rc != 1:
            print("self-test FAILED: bogus symbol was not reported", file=sys.stderr)
            return 1
        print("self-test: bogus symbol correctly reported missing")
        return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--ledger", type=Path, default=None)
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    ledger = args.ledger or (args.repo / "docs" / "api-ledger.md")
    return check(args.repo, ledger, verbose=args.verbose)


if __name__ == "__main__":
    sys.exit(main())
