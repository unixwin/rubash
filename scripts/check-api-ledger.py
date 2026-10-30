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
    python3 scripts/check-api-ledger.py [--repo PATH] [--ledger PATH]
        --niu PATH
    python3 scripts/check-api-ledger.py --self-test

With --niu (ledger §5, completeness half), the script additionally
scans the paired niubash tree for every `rubash::...` path it
consumes and verifies each one is recorded in the ledger: presence
of the recorded symbols alone does not prove the ledger is
complete, and the v2 census missed two consumed families for a
full release cycle before this half existed.

Exit status: 0 when every ledger symbol is present (and, with
--niu, every consumed path is recorded), 1 otherwise.
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


CONSUMED_TOKEN = re.compile(r"rubash::[A-Za-z0-9_:]+")
USE_LIST = re.compile(r"rubash::\{([^{}]*)\}")


def consumed_paths(niu_root):
    """Every `rubash::...` path token niubash's Rust sources name.

    Mirrors the ledger §0 counting rule: occurrences of the token
    pattern in *.rs under crates/ and src/, unioned with the items
    of flat `use rubash::{...}` lists (whose items the bare token
    pattern cannot see). A nested use list degrades to its module
    token, which the module-prefix coverage rule still covers.
    """
    found = set()
    for base in (niu_root / "crates", niu_root / "src"):
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            text = read_all([path])
            for match in CONSUMED_TOKEN.finditer(text):
                token = match.group(0).rstrip(":")
                if token != "rubash":
                    found.add(token)
            for match in USE_LIST.finditer(text):
                for item in match.group(1).split(","):
                    item = item.split(" as ")[0].strip()
                    if item and item != "self":
                        found.add("rubash::" + item)
    return found


def check_completeness(entries, niu_root):
    """Ledger §5.4: everything niubash consumes must be recorded.

    Returns (uncovered, unconsumed_warnings, consumed_count). A
    consumed path is covered when an entry names it exactly, or
    when it is a module prefix of a recorded entry (a module is
    covered by the symbols recorded under it). Recorded-but-
    unconsumed item/variant entries only warn — the ledger
    legitimately carries staged exits — and method entries never
    warn: receiver-form calls are invisible to a path scan.
    """
    recorded = {path for _kind, path in entries}
    consumed = consumed_paths(niu_root)
    uncovered = sorted(
        path
        for path in consumed
        if path not in recorded
        and not any(entry.startswith(path + "::") for entry in recorded)
    )

    def is_used(entry_path):
        return any(
            path == entry_path
            or path.startswith(entry_path + "::")
            or entry_path.startswith(path + "::")
            for path in consumed
        )

    warnings = sorted(
        path
        for kind, path in entries
        if kind in ("item", "variant") and not is_used(path)
    )
    return uncovered, warnings, len(consumed)


def check(repo_root, ledger_path, verbose=False, niu_root=None):
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
    consumed_count = None
    if niu_root is not None:
        uncovered, unconsumed, consumed_count = check_completeness(
            entries, niu_root
        )
        for path in unconsumed:
            print(
                f"check-api-ledger: warning: recorded but not consumed: {path}"
            )
        for path in uncovered:
            missing.append(
                ("consumed", path, "consumed by niubash but not recorded")
            )
    if missing:
        print(f"check-api-ledger: {len(missing)} ledger symbol(s) missing:")
        for kind, path, why in missing:
            print(f"  MISSING {kind} {path} ({why})")
        return 1
    print(f"check-api-ledger: all {len(entries)} ledger symbols present")
    if consumed_count is not None:
        print(
            f"check-api-ledger: all {consumed_count} consumed niubash "
            "paths are recorded"
        )
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
    # Completeness (§5.4): a consumed path nobody recorded must
    # fail; once every consumed path is recorded, it must pass.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text(
            "pub struct Executor;\n", encoding="utf-8"
        )
        ledger = root / "ledger.md"
        ledger.write_text(
            "# t\n\n```api-ledger\nitem rubash::Executor\n```\n",
            encoding="utf-8",
        )
        niu = root / "niu"
        (niu / "src").mkdir(parents=True)
        (niu / "src" / "main.rs").write_text(
            "fn main() {\n"
            "    let _ = rubash::Executor;\n"
            "    let _ = rubash::executor::Executor::new;\n"
            "}\n",
            encoding="utf-8",
        )
        if check(root, ledger, niu_root=niu) != 1:
            print(
                "self-test FAILED: unrecorded consumed path was not reported",
                file=sys.stderr,
            )
            return 1
        (niu / "src" / "main.rs").write_text(
            "fn main() { let _ = rubash::Executor; }\n", encoding="utf-8"
        )
        if check(root, ledger, niu_root=niu) != 0:
            print(
                "self-test FAILED: fully recorded consumption did not pass",
                file=sys.stderr,
            )
            return 1
        print("self-test: completeness check fails unrecorded, passes recorded")
        return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--ledger", type=Path, default=None)
    parser.add_argument(
        "--niu",
        type=Path,
        default=None,
        help="niubash tree: also verify every consumed rubash:: path "
        "is recorded in the ledger (ledger §5.4)",
    )
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    ledger = args.ledger or (args.repo / "docs" / "api-ledger.md")
    return check(args.repo, ledger, verbose=args.verbose, niu_root=args.niu)


if __name__ == "__main__":
    sys.exit(main())
