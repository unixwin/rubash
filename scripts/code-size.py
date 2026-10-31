#!/usr/bin/env python3
"""Net source-size counter (single source of truth for the code-size badge).

Counting rules (see docs/code-size.md in the rubash repo; do not change
them silently -- the badge and the published comparison depend on them):

Rust targets (.rs):
  * Walk the tree, skipping .git/ and target/.
  * A file is in the *tests bucket* (excluded from the headline) when
      - any path component is exactly "tests", or
      - any path component ends with "_tests" (e.g. printf_tests/), or
      - the file name is "tests.rs" or ends with "_tests.rs", or
      - the file is the resolved target of a `#[cfg(test)] mod NAME;`
        declaration (honouring #[path = "..."]).
  * A file is in the *benches/examples bucket* (reported separately,
    not in the headline) when any path component is "benches",
    "benchmarks" or "examples" (and it is not in the tests bucket).
  * Inside every remaining file, `#[cfg(test)] mod ... { ... }` blocks
    (and `mod NAME;` declaration lines) are removed with a lexer-aware
    brace match: braces inside strings, raw strings, char literals and
    comments do not count. Lines touched by a removed range are dropped.
  * The headline number is the count of remaining non-blank lines.
    Physical line count is reported alongside for reference.
  * `#[cfg(test)]` on non-mod items (use/fn/const helpers outside a
    test module) is NOT removed; it is counted and reported as a memo
    (cfg_test_nonmod_items) so the conservatism is visible.

C targets (.c + .h), symmetric rules:
  * tests / examples path components go to their buckets as above.
  * Blank lines are excluded; the headline is non-blank lines.
  * With --pristine DIR: any .c/.h file whose relative path does not
    exist in the pristine release tree is treated as build-generated,
    moved to the "generated" bucket and listed by name. This is how a
    patched *build* tree (e.g. bash 5.3 + patches = 5.3.20, with
    configure output inside) is made comparable to a release tarball.

Usage:
  count_code_size.py [--lang rust|c] [--pristine DIR] [--json OUT]
                     [--write-md FILE] [--badge-out FILE] [--badge-vs LABEL]
                     [LABEL=]PATH ...

Without --lang, a target with a Cargo.toml at its root counts as Rust,
anything else as C. Output is a Markdown table on stdout.
Pure Python 3 stdlib; no third-party packages.
"""

import argparse
import json
import re
import sys
from bisect import bisect_right
from pathlib import Path

SKIP_DIRS = {".git", "target"}
TEST_COMPONENTS_EXACT = {"tests"}
AUX_COMPONENTS = {"benches", "benchmarks", "examples"}

CHAR_RE = re.compile(
    r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'\n])'"
)
CFG_TEST_RE = re.compile(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]")
PATH_ATTR_RE = re.compile(r'path\s*=\s*"([^"]*)"')
WORD_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


# --------------------------------------------------------------------------
# Rust lexer mask: 1 = real code, 0 = string / char / comment content.
# --------------------------------------------------------------------------
def code_mask(text):
    n = len(text)
    mask = bytearray(b"\x01") * n
    i = 0
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = text.find("\n", i)
            if j == -1:
                j = n
            mask[i:j] = b"\x00" * (j - i)
            i = j
        elif c == "/" and nxt == "*":
            depth = 1
            j = i + 2
            while j < n and depth:
                if text[j] == "*" and j + 1 < n and text[j + 1] == "/":
                    depth -= 1
                    j += 2
                elif text[j] == "/" and j + 1 < n and text[j + 1] == "*":
                    depth += 1
                    j += 2
                else:
                    j += 1
            mask[i:j] = b"\x00" * (j - i)
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                if text[j] == "\n":
                    break
                j += 1
            mask[i:j] = b"\x00" * (j - i)
            i = j
        elif c == "'":
            m = CHAR_RE.match(text, i)
            if m:
                mask[i : m.end()] = b"\x00" * (m.end() - i)
                i = m.end()
            else:
                i += 1  # lifetime, stays code
        elif c.isalpha() or c == "_":
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            if text[i:j] in ("r", "br", "rb"):
                k = j
                while k < n and text[k] == "#":
                    k += 1
                if k < n and text[k] == '"':
                    hashes = k - j
                    closer = '"' + "#" * hashes
                    end = text.find(closer, k + 1)
                    end = n if end == -1 else end + len(closer)
                    mask[i:end] = b"\x00" * (end - i)
                    i = end
                    continue
            i = j
        else:
            i += 1
    return mask


def _skip_trivia(text, mask, i):
    """Advance past whitespace and comments (masked regions). Strings are
    NOT skipped: a masked region starting with a quote is significant."""
    n = len(text)
    while i < n:
        if mask[i] == 1:
            if text[i].isspace():
                i += 1
                continue
            return i
        # masked: comment (trivia) or string (significant)
        if text[i] == '"':
            return i
        # comment: skip to its end (next code char)
        while i < n and mask[i] == 0 and text[i] != '"':
            i += 1
    return i


def _read_word(text, mask, i):
    m = WORD_RE.match(text, i)
    if m and mask[i] == 1:
        return m.group(0), m.end()
    return None, i


def _match_delim(text, mask, i, open_ch, close_ch):
    """i is at open_ch (code). Return index just past the matching close."""
    depth = 0
    n = len(text)
    while i < n:
        if mask[i] == 1:
            if text[i] == open_ch:
                depth += 1
            elif text[i] == close_ch:
                depth -= 1
                if depth == 0:
                    return i + 1
        i += 1
    return n


def extract_cfg_test(text, mask):
    """Find #[cfg(test)] items.

    Returns (ranges, decls, nonmod_count):
      ranges     -- char ranges to remove (mod blocks and `mod NAME;` lines)
      decls      -- [(name, path_attr_or_None)] for `mod NAME;` declarations
      nonmod     -- #[cfg(test)] occurrences on non-mod items (kept, memo)
    """
    ranges = []
    decls = []
    nonmod = 0
    n = len(text)
    for m in CFG_TEST_RE.finditer(text):
        if mask[m.start()] != 1:
            continue
        i = m.end()
        path_attr = None
        # Skip any further attributes (capturing #[path = "..."]).
        while True:
            i = _skip_trivia(text, mask, i)
            if i < n and mask[i] == 1 and text[i] == "#":
                end = _match_delim(text, mask, i + 1, "[", "]") if i + 1 < n else n
                inner = text[i:end]
                pm = PATH_ATTR_RE.search(inner)
                if pm:
                    path_attr = pm.group(1)
                i = end
                continue
            break
        i = _skip_trivia(text, mask, i)
        word, wend = _read_word(text, mask, i)
        if word == "pub":
            i = _skip_trivia(text, mask, wend)
            if i < n and mask[i] == 1 and text[i] == "(":
                i = _match_delim(text, mask, i, "(", ")")
                i = _skip_trivia(text, mask, i)
            word, wend = _read_word(text, mask, i)
        if word == "unsafe":
            i = _skip_trivia(text, mask, wend)
            word, wend = _read_word(text, mask, i)
        if word != "mod":
            nonmod += 1
            continue
        i = _skip_trivia(text, mask, wend)
        name, nend = _read_word(text, mask, i)
        if name is None:
            nonmod += 1
            continue
        i = _skip_trivia(text, mask, nend)
        if i < n and mask[i] == 1 and text[i] == ";":
            decls.append((name, path_attr))
            ranges.append((m.start(), i + 1))
        elif i < n and mask[i] == 1 and text[i] == "{":
            end = _match_delim(text, mask, i, "{", "}")
            ranges.append((m.start(), end))
        else:
            nonmod += 1
    # Merge overlapping ranges.
    ranges.sort()
    merged = []
    for a, b in ranges:
        if merged and a <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], b))
        else:
            merged.append((a, b))
    return merged, decls, nonmod


def split_lines(text):
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    starts = []
    pos = 0
    for ln in lines:
        starts.append(pos)
        pos += len(ln) + 1
    return lines, starts


def count_rust_file(text):
    """Return dict(physical, blank, removed, net, nonmod_cfg_test)."""
    mask = code_mask(text)
    ranges, _decls, nonmod = extract_cfg_test(text, mask)
    lines, starts = split_lines(text)
    removed_lines = set()
    for a, b in ranges:
        if b <= a:
            continue
        first = bisect_right(starts, a) - 1
        last = bisect_right(starts, b - 1) - 1
        for k in range(max(first, 0), min(last, len(lines) - 1) + 1):
            removed_lines.add(k)
    net = blank = 0
    for idx, ln in enumerate(lines):
        if idx in removed_lines:
            continue
        if ln.strip():
            net += 1
        else:
            blank += 1
    res = {
        "physical": len(lines),
        "blank": blank,
        "removed": len(removed_lines),
        "net": net,
        "nonmod_cfg_test": nonmod,
    }
    assert res["physical"] == res["net"] + res["blank"] + res["removed"]
    return res


def count_c_file(text):
    lines, _ = split_lines(text)
    net = sum(1 for ln in lines if ln.strip())
    return {"physical": len(lines), "blank": len(lines) - net,
            "removed": 0, "net": net, "nonmod_cfg_test": 0}


def resolve_decl_targets(declaring_file, decls):
    """Resolve `mod NAME;` targets (with #[path]) to existing files."""
    out = set()
    parent = declaring_file.parent
    stem = declaring_file.stem
    is_rootish = stem in ("mod", "lib", "main")
    for name, path_attr in decls:
        cands = []
        if path_attr:
            cands.append(parent / path_attr)
            if not is_rootish:
                cands.append(parent / stem / path_attr)
        elif is_rootish:
            cands.append(parent / f"{name}.rs")
            cands.append(parent / name / "mod.rs")
        else:
            cands.append(parent / stem / f"{name}.rs")
            cands.append(parent / stem / name / "mod.rs")
        for c in cands:
            if c.is_file():
                out.add(c)
                break
    return out


def classify_rust(rel_parts, filename):
    comps = rel_parts[:-1]
    if any(c == "tests" or c.endswith("_tests") for c in comps):
        return "tests"
    if filename == "tests.rs" or filename.endswith("_tests.rs"):
        return "tests"
    if any(c in AUX_COMPONENTS for c in comps):
        return "aux"
    return "main"


def count_rust_tree(root):
    root = Path(root)
    files = []
    for p in sorted(root.rglob("*.rs")):
        rel = p.relative_to(root)
        if any(part in SKIP_DIRS for part in rel.parts):
            continue
        files.append((p, rel))
    # First pass: collect cfg(test) mod-declaration targets.
    decl_targets = set()
    texts = {}
    for p, rel in files:
        text = p.read_text(encoding="utf-8", errors="replace")
        texts[p] = text
        mask = code_mask(text)
        _r, decls, _n = extract_cfg_test(text, mask)
        if decls:
            decl_targets |= resolve_decl_targets(p, decls)
    buckets = {"main": [], "tests": [], "aux": []}
    for p, rel in files:
        bucket = classify_rust(rel.parts, rel.name)
        if p in decl_targets:
            bucket = "tests"
        buckets[bucket].append((p, rel))
    detail = {"buckets": {}, "files": []}
    for bucket, items in buckets.items():
        agg = {"files": len(items), "physical": 0, "blank": 0,
               "removed": 0, "net": 0, "nonmod_cfg_test": 0}
        for p, rel in items:
            r = count_rust_file(texts[p])
            for k in agg:
                if k != "files":
                    agg[k] += r[k]
            detail["files"].append({"path": str(rel), "bucket": bucket, **r})
        detail["buckets"][bucket] = agg
    return detail


def count_c_tree(root, pristine=None):
    root = Path(root)
    pristine_set = None
    if pristine is not None:
        pristine = Path(pristine)
        pristine_set = set()
        for p in pristine.rglob("*"):
            if p.suffix in (".c", ".h") and p.is_file():
                rel = p.relative_to(pristine)
                if not any(part in SKIP_DIRS for part in rel.parts):
                    pristine_set.add(str(rel))
    buckets = {"main": [], "tests": [], "aux": [], "generated": []}
    generated_names = []
    for p in sorted(root.rglob("*")):
        if not p.is_file() or p.suffix not in (".c", ".h"):
            continue
        rel = p.relative_to(root)
        if any(part in SKIP_DIRS for part in rel.parts):
            continue
        comps = rel.parts[:-1]
        if any(c == "tests" or c.endswith("_tests") for c in comps):
            bucket = "tests"
        elif any(c in AUX_COMPONENTS for c in comps):
            bucket = "aux"
        elif pristine_set is not None and str(rel) not in pristine_set:
            bucket = "generated"
            generated_names.append(str(rel))
        else:
            bucket = "main"
        buckets[bucket].append((p, rel))
    detail = {"buckets": {}, "files": [], "generated_files": generated_names}
    for bucket, items in buckets.items():
        agg = {"files": 0, "physical": 0, "blank": 0, "removed": 0, "net": 0,
               "nonmod_cfg_test": 0}
        for p, rel in items:
            text = p.read_text(encoding="utf-8", errors="replace")
            r = count_c_file(text)
            agg["files"] += 1
            for k in ("physical", "blank", "removed", "net"):
                agg[k] += r[k]
            detail["files"].append({"path": str(rel), "bucket": bucket, **r})
        detail["buckets"][bucket] = agg
    return detail


def fmt(n):
    return f"{n:,}"


def render_markdown(results):
    out = []
    out.append("| 目标 | 语言 | 文件 | 物理行 | 空行 | 剔除行 | 净行数（头条） |")
    out.append("|---|---|---:|---:|---:|---:|---:|")
    for label, lang, d in results:
        m = d["buckets"]["main"]
        out.append(
            f"| {label} | {lang} | {m['files']} | {fmt(m['physical'])} | "
            f"{fmt(m['blank'])} | {fmt(m['removed'])} | **{fmt(m['net'])}** |"
        )
    out.append("")
    out.append("分桶明细（不进头条）:")
    out.append("")
    for label, lang, d in results:
        for bucket in ("tests", "aux", "generated"):
            if bucket in d["buckets"] and d["buckets"][bucket]["files"]:
                b = d["buckets"][bucket]
                out.append(
                    f"- {label} · {bucket}: {b['files']} 文件, "
                    f"物理 {fmt(b['physical'])} 行, 净 {fmt(b['net'])} 行"
                )
        if "generated_files" in d and d["generated_files"]:
            out.append(
                f"- {label} · 生成文件清单: {', '.join(d['generated_files'])}"
            )
        memo = d["buckets"]["main"]["nonmod_cfg_test"]
        if memo:
            out.append(
                f"- {label} · 备忘: 主桶内有 {memo} 处 #[cfg(test)] 非 mod 项"
                "（按口径不剔除, 计入头条）"
            )
    return "\n".join(out) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description="Net source-size counter")
    ap.add_argument("targets", nargs="+", help="[LABEL=]PATH ...")
    ap.add_argument("--lang", choices=("rust", "c"), default=None)
    ap.add_argument("--pristine", default=None,
                    help="pristine release tree (C targets: file-set filter)")
    ap.add_argument("--json", default=None, help="write full JSON detail")
    ap.add_argument("--write-md", default=None,
                    help="splice the table into FILE between code-size markers")
    ap.add_argument("--badge-out", default=None,
                    help="write a shields.io endpoint JSON for the 1st target")
    ap.add_argument("--badge-vs", default=None,
                    help="label of the target to ratio against in the badge")
    args = ap.parse_args(argv)

    results = []
    for t in args.targets:
        if "=" in t and not Path(t).exists():
            label, path = t.split("=", 1)
        else:
            label, path = Path(t).name, t
        root = Path(path)
        lang = args.lang
        if lang is None:
            lang = "rust" if (root / "Cargo.toml").exists() else "c"
        if lang == "rust":
            d = count_rust_tree(root)
        else:
            d = count_c_tree(root, pristine=args.pristine)
        results.append((label, lang, d))

    table = render_markdown(results)
    sys.stdout.write(table)

    if args.json:
        payload = {
            "rules_version": 1,
            "targets": [
                {"label": label, "lang": lang, **d} for label, lang, d in results
            ],
        }
        Path(args.json).write_text(
            json.dumps(payload, ensure_ascii=False, indent=1) + "\n",
            encoding="utf-8",
        )
    if args.write_md:
        p = Path(args.write_md)
        doc = p.read_text(encoding="utf-8")
        begin = "<!-- BEGIN code-size (generated by scripts/code-size.py) -->"
        end = "<!-- END code-size -->"
        if begin not in doc or end not in doc:
            raise SystemExit(f"markers not found in {p}")
        head, rest = doc.split(begin, 1)
        _old, tail = rest.split(end, 1)
        p.write_text(head + begin + "\n\n" + table + end + tail,
                     encoding="utf-8")
    if args.badge_out:
        label0, _lang0, d0 = results[0]
        net = d0["buckets"]["main"]["net"]
        message = f"{fmt(net)} lines"
        if args.badge_vs:
            for label, _lang, d in results:
                if label == args.badge_vs:
                    gnu = d["buckets"]["main"]["net"]
                    if gnu:
                        message += f" ({net / gnu:.2f}x GNU bash)"
                    break
        badge = {
            "schemaVersion": 1,
            "label": "net code size",
            "message": message,
            "color": "blue",
        }
        Path(args.badge_out).write_text(
            json.dumps(badge, ensure_ascii=False, indent=1) + "\n",
            encoding="utf-8",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
