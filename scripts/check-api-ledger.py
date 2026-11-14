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
this tree. This script is deliberately a tripwire, not a type checker, but it
does check that each recorded PATH is the path the symbol is actually
exported under (rubash#2847): the module chain must be `pub mod`
declarations (or `pub use` module re-exports) from src/lib.rs, and the
final name must be declared `pub` at that module's top level or
re-exported there by `pub use` (named, renamed, or through a glob).
A symbol that exists only deeper in the subtree, or behind a private
module, no longer passes on presence alone.

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


# ---------------------------------------------------------------------------
# Export-path resolver (rubash#2847).
#
# A ledger path `rubash::a::b::Name` is only true when a consumer can
# write it: every module step must be externally visible (`pub mod`, or
# a `pub use` that re-exports a module), and `Name` must be `pub` at the
# top level of the final module or re-exported there. The resolver is a
# source-level approximation of rustc's name resolution — comments and
# string/char literals are blanked, brace depth decides "top level",
# and only bare `pub` counts (`pub(crate)` is not part of the surface).
# It deliberately does not chase macro-generated items; a ledger entry
# that needs one fails loudly and must be resolved by hand.
# ---------------------------------------------------------------------------

_DECL_ITEM = re.compile(
    r"\bpub\s+(?:(?:const|async|unsafe|extern\s*(?:\"[^\"]*\")?)\s+)*"
    r"(fn|struct|enum|trait|type|const|static|union|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
_PUB_USE = re.compile(r"\bpub\s+use\s+([^;]*);")
_PATH_ATTR = re.compile(r"#\s*\[\s*path\s*=\s*\"([^\"]+)\"\s*\]\s*$")


def _blank_literals(text):
    """Blank comments and string/char literals, keeping offsets and newlines."""
    out = list(text)
    i = 0
    n = len(text)

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and re.match(r'r#*"', text[i:]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            close = '"' + hashes
            j = text.find(close, i + 2 + len(hashes))
            j = n if j < 0 else j + len(close)
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            blank(i, min(j + 1, n))
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'", text[i:])
            if m:
                blank(i, i + m.end())
                i += m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


class _Module:
    """One module body: its blanked text span and where children live."""

    def __init__(self, path, text, child_dir, start=0, end=None, raw=None):
        self.path = path
        self.text = text
        self.raw = text if raw is None else raw
        self.child_dir = child_dir
        self.start = start
        self.end = len(text) if end is None else end
        self.top = None


def _top_level_starts(module):
    """Offsets inside `module` whose brace depth is the module's own top level."""
    if module.top is None:
        top, depth = bytearray(len(module.text)), 0
        for i in range(module.start, module.end):
            c = module.text[i]
            if c == "}":
                depth -= 1
            if depth == 0:
                top[i] = 1
            if c == "{":
                depth += 1
        module.top = top
    return module.top


def _top_level_matches(module, pattern):
    """Matches of `pattern` in `module` that start at its top level."""
    top = _top_level_starts(module)
    return [
        match
        for match in pattern.finditer(module.text, module.start, module.end)
        if top[match.start()]
    ]


def _matching_brace(text, open_index):
    depth = 0
    for i in range(open_index, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    return len(text)


_FILE_CACHE = {}


def _load_module_file(path, child_dir):
    key = (str(path), str(child_dir))
    if key not in _FILE_CACHE:
        try:
            raw = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            _FILE_CACHE[key] = None
            return None
        _FILE_CACHE[key] = _Module(path, _blank_literals(raw), child_dir, raw=raw)
    return _FILE_CACHE[key]


def _crate_root(src_root):
    return _load_module_file(src_root / "lib.rs", src_root)


def _use_tree_entries(tree):
    """Flatten a `use` tree into (path_segments, exported_name) pairs.

    `exported_name` is the visible name (`as` rename applied), or `*`
    for a glob; `self` inside a brace list exports the prefix's last
    segment.
    """
    tree = re.sub(r"\s+", " ", tree.strip())
    tree = re.sub(r"\s*(::|\{|\}|,)\s*", r"\1", tree)
    results = []

    def split_top(body):
        parts, depth, cur = [], 0, ""
        for ch in body:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            if ch == "," and depth == 0:
                parts.append(cur)
                cur = ""
            else:
                cur += ch
        if cur:
            parts.append(cur)
        return parts

    def walk(prefix, item):
        if not item:
            return
        if "{" in item:
            head, _, rest = item.partition("{")
            head_segs = [seg for seg in head.split("::") if seg]
            for part in split_top(rest[: rest.rfind("}")]):
                walk(prefix + head_segs, part)
            return
        renamed = re.fullmatch(r"(\S+) as ([A-Za-z_][A-Za-z0-9_]*)", item.strip())
        if renamed:
            name, alias = renamed.groups()
        else:
            name, alias = item.strip(), ""
        segs = prefix + [seg for seg in name.split("::") if seg]
        if not segs:
            return
        if segs[-1] == "*":
            results.append((segs[:-1], "*"))
            return
        if segs[-1] == "self":
            segs = segs[:-1]
            if not segs:
                return
        results.append((segs, alias or segs[-1]))

    walk([], tree)
    return results


def _module_decls(module):
    """Top-level `pub` declarations and `pub use` entries of a module."""
    if getattr(module, "decls", None) is None:
        decls = {}
        for match in _top_level_matches(module, _DECL_ITEM):
            decls.setdefault(match.group(2), match.group(1))
        uses = []
        for match in _top_level_matches(module, _PUB_USE):
            uses.extend(_use_tree_entries(match.group(1)))
        module.decls = (decls, uses)
    return module.decls


def _child_module(module, name, require_pub=True):
    """The `mod name` child of `module`, inline or file-backed.

    The ledger path itself may only step through `pub mod` (a consumer
    cannot name a private module); a `pub use` target inside the crate
    may name any module, because the re-export is what publishes it.
    A `#[path = "..."]` attribute on the line above the declaration is
    honoured relative to the declaring file's directory.
    """
    text = module.text
    if require_pub:
        visibility = r"\bpub\s+mod\s+"
    else:
        visibility = r"(?:\bpub\s*(?:\([^)]*\)\s*)?)?\bmod\s+"
    pattern = re.compile(visibility + re.escape(name) + r"\s*([;{])")
    for match in _top_level_matches(module, pattern):
        if match.group(1) == "{":
            open_index = match.end() - 1
            close = _matching_brace(text, open_index)
            return _Module(
                module.path,
                text,
                module.child_dir / name,
                open_index + 1,
                close,
                raw=module.raw,
            )
        line_start = module.raw.rfind("\n", 0, match.start())
        previous = module.raw[: max(line_start, 0)].rstrip().splitlines()[-1:]
        for attr in previous:
            path_attr = _PATH_ATTR.search(attr.strip())
            if path_attr:
                target = module.path.parent / path_attr.group(1)
                return _load_module_file(target, target.parent / target.stem)
        for candidate in (
            module.child_dir / (name + ".rs"),
            module.child_dir / name / "mod.rs",
        ):
            if candidate.exists():
                return _load_module_file(candidate, module.child_dir / name)
        return None
    return None


def _resolve_module(src_root, start, segments, parents, seen=None, require_pub=True):
    """Resolve a module path from `start`; None when not reachable.

    `parents` is the chain of enclosing modules of `start` (for
    `super::`). Steps go through `pub mod` children or through a
    `pub use` that re-exports a module under the step's name.
    """
    seen = set() if seen is None else seen
    module, chain = start, list(parents)
    for index, seg in enumerate(segments):
        if seg == "crate":
            module, chain = _crate_root(src_root), []
            continue
        if seg == "self":
            continue
        if seg == "super":
            if not chain:
                return None, []
            module = chain.pop()
            continue
        child = _child_module(module, seg, require_pub)
        if child is None:
            child_chain = None
            for use_segs, exported in _module_decls(module)[1]:
                if exported != seg:
                    continue
                key = (str(module.path), module.start, tuple(use_segs))
                if key in seen:
                    continue
                seen.add(key)
                target, target_chain = _resolve_module(
                    src_root, module, use_segs, chain, seen, require_pub=False
                )
                if target is not None:
                    child, child_chain = target, target_chain
                    break
            if child is None:
                return None, []
            chain = child_chain
            module = child
            continue
        chain.append(module)
        module = child
    return module, chain


def _module_exports(src_root, module, chain, name, seen=None):
    """True when `name` is a public item of `module` (decl or re-export)."""
    seen = set() if seen is None else seen
    key = (str(module.path), module.start, name)
    if key in seen:
        return False
    seen.add(key)
    decls, uses = _module_decls(module)
    if name in decls:
        return True
    for use_segs, exported in uses:
        if exported == name:
            return True
    for use_segs, exported in uses:
        if exported != "*":
            continue
        target, target_chain = _resolve_module(
            src_root, module, use_segs, chain, require_pub=False
        )
        if target is not None and _module_exports(
            src_root, target, target_chain, name, seen
        ):
            return True
    return False


def export_path_resolves(src_root, segments):
    """segments: full path after `rubash::`; True when it is a real export."""
    root = _crate_root(src_root)
    if root is None:
        return False
    module, chain = _resolve_module(src_root, root, segments[:-1], [])
    if module is None:
        return False
    return _module_exports(src_root, module, chain, segments[-1])


def declared_anywhere(src_root, name):
    """Presence by declaration pattern anywhere under src/ (diagnostic only).

    Used to tell "recorded under the wrong path" apart from "gone":
    the pre-#2847 tripwire accepted presence like this as proof.
    """
    text = read_all(sorted(src_root.rglob("*.rs")))
    return any(
        re.search(pattern.format(name=re.escape(name)), text)
        for pattern in PUB_DECL_PATTERNS
    )


def item_exists(src_root, segments):
    """segments: full path after `rubash::`, last segment is the item.

    The recorded path itself must resolve (rubash#2847): presence of a
    same-named declaration elsewhere in the module's subtree is not
    enough.
    """
    return export_path_resolves(src_root, segments)


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
    # The receiver type's recorded path must itself be a real export
    # (rubash#2847); a method under a wrong type path is unusable.
    if len(segments) < 2 or not item_exists(src_root, segments[:-1]):
        return False
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
            # Name the failing step: for a method/variant the owner
            # type's path is what must resolve.
            owner = segments if kind == "item" else segments[:-1]
            if kind != "item" and item_exists(src_root, owner):
                why = "not found in tree"
            elif declared_anywhere(src_root, owner[-1]):
                why = (
                    "declared in the tree but not exported under "
                    f"rubash::{'::'.join(owner)}"
                )
            else:
                why = "not found in tree"
            missing.append((kind, path, why))
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
    # Export paths (rubash#2847): a symbol that exists in the subtree
    # but is not re-exported where the ledger says, or sits behind a
    # private module, must fail; its real export paths must pass.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "src" / "api").mkdir(parents=True)
        (root / "src" / "lib.rs").write_text(
            "pub mod api;\nmod hidden;\npub use hidden::Shown;\n", encoding="utf-8"
        )
        (root / "src" / "hidden.rs").write_text(
            "pub struct Shown;\npub struct Hidden;\n", encoding="utf-8"
        )
        (root / "src" / "api" / "mod.rs").write_text(
            "pub mod capabilities;\nmod nodes;\npub use nodes::*;\n"
            "pub use capabilities::{\n    Handler,\n};\n"
            "pub use capabilities::Handler as Renamed;\n"
            "#[path = \"shim_impl.rs\"]\npub mod shim;\n",
            encoding="utf-8",
        )
        (root / "src" / "api" / "shim_impl.rs").write_text(
            "pub fn shimmed() {}\n", encoding="utf-8"
        )
        (root / "src" / "api" / "capabilities.rs").write_text(
            "pub trait Handler {}\npub enum CandidateKind {\n    File,\n    Directory,\n}\n"
            "impl CandidateKind {\n    pub fn label(&self) {}\n}\n",
            encoding="utf-8",
        )
        (root / "src" / "api" / "nodes.rs").write_text(
            "pub struct Node;\n", encoding="utf-8"
        )
        ledger = root / "ledger.md"

        def run(lines):
            ledger.write_text(
                "# t\n\n```api-ledger\n" + "\n".join(lines) + "\n```\n",
                encoding="utf-8",
            )
            return check(root, ledger)

        good = [
            "item rubash::api::Handler",
            "item rubash::api::capabilities::CandidateKind",
            "variant rubash::api::capabilities::CandidateKind::File",
            "method rubash::api::capabilities::CandidateKind::label",
            "item rubash::api::Node",
            "item rubash::api::Renamed",
            "item rubash::api::shim::shimmed",
            "item rubash::Shown",
        ]
        if run(good) != 0:
            print("self-test FAILED: real export paths were rejected", file=sys.stderr)
            return 1
        for bad in (
            "item rubash::api::CandidateKind",
            "variant rubash::api::CandidateKind::File",
            "method rubash::api::CandidateKind::label",
            "item rubash::api::nodes::Node",
            "item rubash::hidden::Shown",
            "item rubash::Hidden",
            "item rubash::api::capabilities::Renamed",
        ):
            if run(good + [bad]) != 1:
                print(
                    f"self-test FAILED: unexported path passed: {bad}",
                    file=sys.stderr,
                )
                return 1
        print("self-test: unexported ledger paths correctly reported")
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
