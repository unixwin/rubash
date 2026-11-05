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
full release cycle before this half existed. The same mode also
runs the Session-method census (ledger §5.5): it enumerates every
`pub fn` in the engine's `impl Session` block and scans niubash
for receiver-form calls of each name, because a receiver-form
call names no `rubash::` path and is invisible to the path scan
— the blind spot that left three consumed Session faces
unrecorded until the GAP-13 recount wave.

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
    covered by the symbols recorded under it). A trailing
    `::default` segment names the type's `Default` trait impl, not
    a freestanding item: the ledger records it under the type's
    `item` entry (a trait constructor has no `pub fn` the `method`
    kind could name), so a consumed `T::default` is covered when
    `T` is covered. Recorded-but-unconsumed item/variant entries
    only warn — the ledger legitimately carries staged exits —
    and method entries never warn: receiver-form calls are
    invisible to a path scan.
    """
    recorded = {path for _kind, path in entries}
    consumed = consumed_paths(niu_root)

    def is_covered(path):
        if path in recorded:
            return True
        if any(entry.startswith(path + "::") for entry in recorded):
            return True
        if path.endswith("::default"):
            return is_covered(path[: -len("::default")])
        return False

    uncovered = sorted(path for path in consumed if not is_covered(path))

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


# Ledger §5.5: Session-method names whose receiver-form calls in
# niubash collide with unrelated calls on other types, so the
# census cannot decide them by name alone. Each entry is a
# hand verification, recorded here — the list grows only with
# one. Founding entry: `parse` — niubash calls `.parse(` as
# `FromStr::parse` on strings and numbers, never as
# `Session::parse` (the engine method exists but has no host
# consumer); the scan reports its hits as a warning, not a
# failure, and a human re-verifies the shape on each census.
AMBIGUOUS_SESSION_METHODS = frozenset({"parse"})

_IMPL_SESSION = re.compile(r"impl\s+Session\s*\{")
_PUB_FN = re.compile(r"pub\s+(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*(?:<[^>]*>)?\s*\(")


def session_methods(src_root):
    """Every `pub fn` name in the engine's `impl Session` block(s).

    Brace-matched from each `impl Session {` opener: impl blocks
    for a type are spread over files in this codebase, and the
    census needs the declared set, not a single file's. The
    `Session` spelling is exact — `SessionError`,
    `SessionBuilder`, `SessionDriver` and `SessionHistory` are
    different types and never match.
    """
    names = set()
    for path in sorted(src_root.rglob("*.rs")):
        text = read_all([path])
        for match in _IMPL_SESSION.finditer(text):
            depth = 1
            i = match.end()
            while i < len(text) and depth > 0:
                char = text[i]
                if char == "{":
                    depth += 1
                elif char == "}":
                    depth -= 1
                i += 1
            block = text[match.end() : i - 1]
            names.update(_PUB_FN.findall(block))
    return names


def session_method_calls(niu_root, names):
    """Receiver-form calls of each Session method name in niubash.

    A call is `.name(` on any receiver (a session / guard lend,
    or any other value — the name alone cannot tell them apart)
    or the `Session::name(` associated spelling. Returns
    {name: [(file, line), ...]}. Guard methods
    (`CompletionAttempt`, `Finish`) are not enumerated here:
    their calls ride under their opener rows in §2.3.
    """
    patterns = {
        name: (
            re.compile(r"\." + re.escape(name) + r"\s*\("),
            re.compile(r"\bSession::" + re.escape(name) + r"\s*\("),
        )
        for name in names
    }
    hits = {name: [] for name in names}
    for base in (niu_root / "crates", niu_root / "src"):
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            try:
                lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            except OSError:
                continue
            for lineno, line in enumerate(lines, 1):
                for name, (receiver, associated) in patterns.items():
                    if receiver.search(line) or associated.search(line):
                        hits[name].append((str(path), lineno))
    return hits


def check_session_method_census(entries, niu_root, src_root):
    """Ledger §5.5: every consumed Session method must be recorded.

    Returns (missing, ambiguous_warnings, declared_count,
    called_count). A declared Session method with receiver-form
    calls in niubash but no §6 `method rubash::api::Session::<name>`
    entry fails, unless the name is on the ambiguous list — then
    it warns, and a human re-verifies the call shape. A declared
    method with no calls in niubash is fine either way: the ledger
    legitimately carries staged faces with no consumer yet.
    """
    recorded = {
        path.split("::")[-1]
        for kind, path in entries
        if kind == "method" and path.startswith("rubash::api::Session::")
    }
    declared = session_methods(src_root)
    calls = session_method_calls(niu_root, declared)
    missing = []
    ambiguous = []
    called = 0
    for name in sorted(declared):
        sites = calls.get(name, [])
        if not sites:
            continue
        called += 1
        if name in recorded:
            continue
        if name in AMBIGUOUS_SESSION_METHODS:
            ambiguous.append((name, sites))
        else:
            missing.append((name, sites))
    return missing, ambiguous, len(declared), called


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
        census_missing, census_ambiguous, declared_count, called_count = (
            check_session_method_census(entries, niu_root, src_root)
        )
        for name, sites in census_ambiguous:
            print(
                f"check-api-ledger: warning: ambiguous Session method "
                f"{name} called {len(sites)} time(s) in niubash "
                f"(hand-verify the call shape; first: "
                f"{sites[0][0]}:{sites[0][1]})"
            )
        for name, sites in census_missing:
            missing.append(
                (
                    "session-method",
                    f"rubash::api::Session::{name}",
                    f"called {len(sites)} time(s) by niubash but not "
                    f"recorded (first: {sites[0][0]}:{sites[0][1]})",
                )
            )
        if verbose:
            print(
                f"check-api-ledger: Session census: {declared_count} "
                f"declared methods, {called_count} called by niubash"
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
    # Session-method census (§5.5): a consumed-but-unrecorded
    # Session method must fail; recording it must pass; an
    # ambiguous name must warn but pass.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text(
            "pub mod api;\n", encoding="utf-8"
        )
        (root / "src" / "api.rs").write_text(
            "pub struct Session;\n"
            "impl Session {\n"
            "    pub fn glob_filename(&self) {}\n"
            "    pub fn parse(&mut self) {}\n"
            "}\n",
            encoding="utf-8",
        )
        ledger = root / "ledger.md"
        ledger.write_text(
            "# t\n\n```api-ledger\n"
            "item rubash::api::Session\n"
            "method rubash::api::Session::parse\n"
            "```\n",
            encoding="utf-8",
        )
        niu = root / "niu"
        (niu / "src").mkdir(parents=True)
        (niu / "src" / "main.rs").write_text(
            "fn main() {\n"
            "    let _ = session().glob_filename();\n"
            "    let _ = \"1\".parse().unwrap();\n"
            "}\n",
            encoding="utf-8",
        )
        if check(root, ledger, niu_root=niu) != 1:
            print(
                "self-test FAILED: unrecorded consumed Session method "
                "was not reported",
                file=sys.stderr,
            )
            return 1
        ledger.write_text(
            "# t\n\n```api-ledger\n"
            "item rubash::api::Session\n"
            "method rubash::api::Session::glob_filename\n"
            "```\n",
            encoding="utf-8",
        )
        # `parse` is deliberately NOT recorded here: its `.parse(`
        # hits are the unrelated `FromStr::parse` calls, so the
        # census must warn on the ambiguous name — not fail — and
        # the run must still pass.
        if check(root, ledger, niu_root=niu) != 0:
            print(
                "self-test FAILED: ambiguous `parse` calls did not "
                "pass as a warning",
                file=sys.stderr,
            )
            return 1
        print(
            "self-test: Session census fails unrecorded, passes "
            "recorded, warns on ambiguous"
        )
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
