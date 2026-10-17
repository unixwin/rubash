#!/usr/bin/env python3
"""Differential harness for rubash in-process fast paths.

rubash runs some commands in-process ("fast path") when the argument shape is
simple, and falls back to the external WinuxCmd binary otherwise.  The fast
path is deliberately invisible to `type`/`command -v`, so a divergence between
the two implementations silently changes the result of `a | cmd` depending on
whether the producer is a builtin.

This harness fires every supported fast-path shape through three producers and
compares **stdout and exit status** byte for byte.  stderr is additionally
compared for the four cases that are expected to print a diagnostic
(`sleep_missing`, `sleep_invalid`, `sleep_badunit`, `basename_extra`; the
`stderr=True` flag).  Appendix A marks the stderr-compared rows `yes`.

  FAST    niu -c '<body>'                       (in-process fast path)
  EXT     <WX_BIN>/<cmd>.exe                    (external winuxcmd fallback)
  GNU     D:/Git/bin/bash.exe -c '<body>'       (GNU coreutils reference)

The primary signal is FAST vs EXT (the fast path is supposed to be a pure
optimisation of the fallback).  GNU is reported for classification.  A case
"passes" when FAST matches EXT byte for byte on stdout and on exit status (and
on stderr when the case is flagged).

Why a Python script instead of a Rust integration test: the matrix needs to
spawn the installed `niu.exe`, WinuxCmd binaries and the Git-for-Windows GNU
coreutils, and to compare raw child stdout bytes.  That is an out-of-process
differential; a Rust #[test] would have to re-implement the process plumbing
for no benefit and would only run on a machine with this exact tool layout.
The script is safe to re-run (and to run concurrently: fixtures are
per-process) and takes `--only`, `--list`, `--json`, `--no-ext`.

Exit status:
  0  every non-known case matches its fallback (and every known divergence
     still diverges -- a case that mysteriously starts passing is reported so
     expectations can be updated)
  1  at least one case regressed, a known divergence no longer diverges, or
     `--only` matched no cases
  2  a required toolchain is missing (see `NIU`, `WX_BIN`, `GNU_BASH` below)

Add a case by appending one dict to CASES; `known` marks a confirmed,
issue-tracked divergence that is expected to fail.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

NIU = os.environ.get("NIU", r"C:\Users\cmx\AppData\Local\Programs\Niubash\niu.exe")
GNU_BASH = os.environ.get("GNU_BASH", r"D:\Git\bin\bash.exe")
GNU_PATH = os.environ.get("GNU_PATH", r"D:\Git\usr\bin;D:\Git\bin")
WX_BIN = os.environ.get(
    "WX_BIN", r"C:\Users\cmx\AppData\Local\Programs\Niubash\winuxcmd\usr\bin"
)
TMPDIR = os.environ.get("AUDIT_TMP", r"C:\Users\cmx\AppData\Local\Temp\opencode")

# The fast path's own fallback (execute_external_pipeline_stage /
# execute_external) launches the WinuxCmd binary by name from PATH.  Referencing
# that binary by absolute path defeats both the named inline arm and the
# `command` builtin's own dispatch, reproducing the exact fallback byte
# handling.  (Spot check: `... | command sed s/a/A/` mangles carried control
# bytes on rubash master, while the absolute winuxcmd path and the declined
# fast-path fallback are correct -- so `command` is NOT a faithful reference.)
EXT_EXE = {
    "true": "true.exe",
    "false": "false.exe",
    "echo": "echo.exe",
    "printf": "printf.exe",
    "head": "head.exe",
    "cat": "cat.exe",
    "sed": "sed.exe",
    "grep": "grep.exe",
    "wc": "wc.exe",
    "tr": "tr.exe",
    "sort": "sort.exe",
    "tail": "tail.exe",
    "uniq": "uniq.exe",
    "sleep": "sleep.exe",
    "dirname": "dirname.exe",
    "basename": "basename.exe",
    "env": "env.exe",
    "uname": "uname.exe",
    "arch": "arch.exe",
}
# Commands with no external counterpart: compare against the GNU bash builtin.
NO_EXT = {":", "trap"}

# Set by --no-ext: run without any WinuxCmd reference (every case compares
# against GNU instead).  When false, a missing WX_BIN is a fatal error rather
# than a silent downgrade to the GNU reference.
EXT_DISABLED = False


def ext_cmd(prog):
    if EXT_DISABLED:
        return None
    exe = EXT_EXE.get(prog)
    if not exe or prog in NO_EXT:
        return None
    path = os.path.join(WX_BIN, exe)
    if not os.path.exists(path):
        return None
    return "'%s'" % path.replace("\\", "/")

# Kinds:
#   pipe   producer | CMD args
#   subst  x=$(producer | CMD args); rc=$?; printf '%s' "$x"; exit $rc
#   plain  CMD args            (top-level dispatch fast path, no pipeline)


def octalfmt(data: bytes) -> str:
    """Encode bytes as a printf octal format string (3 octal digits per byte)."""
    return "".join("\\%03o" % b for b in data)


PLAIN = b"alpha\nbeta\ngamma\n"
NOEOL = b"alpha\nbeta\ngamma"
EMPTY = b""
CTRL = b"a\x1bb\nc\n"
FF = b"a\x0cb\n"
CRLF = b"a\r\nb\r\n"
NUL = b"a\x00b\n"
TABS = b"a\tb  c\n"
BLANKS = b"a\n\n\nb\n"
UTF8 = "café\nαβγ\n日本語\n".encode("utf-8")
DOT = b"axb\n"
LARGE_CMD = (b"x" * 4000 + b"\n")


def P(data):
    return "printf '%s'" % octalfmt(data)


_LARGE_FILE = None
_FIXTURES = {}


def large_producer():
    return "cat '{LARGE}'"


def resolve_placeholders(text):
    if not text:
        return text
    for key, path in _FIXTURES.items():
        text = text.replace("{%s}" % key, path.replace("\\", "/"))
    text = text.replace("{LARGE}", (_LARGE_FILE or "").replace("\\", "/"))
    return text


CASES = []


def add(cid, kind, prog, cmd_args, producer=None, known=None, note="", stderr=False,
        informational=False, env=None, require_empty_stderr=False):
    CASES.append(
        {
            "id": cid,
            "kind": kind,
            "prog": prog,
            "args": cmd_args,
            "producer": producer,
            "known": known,
            "note": note,
            "stderr": stderr,
            "informational": informational,
            "env": env,
            "require_empty_stderr": require_empty_stderr,
        }
    )


# ---------------------------------------------------------------- true/: /false
add("true_plain", "pipe", "true", "", P(PLAIN), note="null command, rc 0")
add("colon_plain", "pipe", ":", "", P(PLAIN), note="null command, rc 0")
add("false_plain", "pipe", "false", "", P(PLAIN), note="rc 1, no output")
add("true_args", "pipe", "true", "-x", P(PLAIN), note="true accepts/ignores args")

# ---------------------------------------------------------------- echo
add("echo_basic", "pipe", "echo", "hello world", P(PLAIN))
add("echo_n", "pipe", "echo", "-n hi", P(PLAIN), note="builtin vs winuxcmd echo")
add("echo_e", "pipe", "echo", "-e 'a\\tb\\n'", P(PLAIN), note="escape decoding")

# ---------------------------------------------------------------- printf
add("printf_s", "pipe", "printf", "'%s-%s\\n' a b", P(PLAIN))
add("printf_esc", "pipe", "printf", "'x\\ty\\n'", P(PLAIN))

# ---------------------------------------------------------------- trap
add("trap_list", "pipe", "trap", "", P(PLAIN), note="shell state, informational", informational=True)
add("trap_l", "pipe", "trap", "-l", P(PLAIN), note="signal table is platform-specific", informational=True)

# ---------------------------------------------------------------- head (pipe)
add("head_default", "pipe", "head", "", P(PLAIN))
add("head_n2", "pipe", "head", "-n 2", P(PLAIN))
add("head_n2join", "pipe", "head", "-n2", P(PLAIN))
add("head_long", "pipe", "head", "--lines=2", P(PLAIN))
add("head_dash2", "pipe", "head", "-2", P(PLAIN))
add("head_n0", "pipe", "head", "-n 0", P(PLAIN))
add(
    "head_negn1",
    "pipe",
    "head",
    "-n -1",
    P(PLAIN),
    known="head_line_count parses -n -1 as None -> default 10 instead of all-but-last",
    note="GNU -n -1 = all but last line",
)
add("head_c", "pipe", "head", "-c 3", P(PLAIN), note="byte mode falls back")
add("head_q", "pipe", "head", "-q", P(PLAIN))
add(
    "head_badnum",
    "pipe",
    "head",
    "-n abc",
    P(PLAIN),
    known="rubash#708: invalid number silently treated as default 10",
    note="flag parsing: accepted, rc 0 vs error rc 1",
)
add(
    "head_unknown",
    "pipe",
    "head",
    "-x",
    P(PLAIN),
    known="rubash#708: unknown option silently ignored, defaults to 10 lines",
    note="flag parsing",
)
add(
    "head_missing_arg",
    "pipe",
    "head",
    "-n",
    P(PLAIN),
    known="rubash#708: missing -n operand silently treated as default 10",
    note="flag parsing",
)
add(
    "head_long_missing",
    "pipe",
    "head",
    "--lines",
    P(PLAIN),
    known="rubash#708: missing --lines operand silently treated as default 10",
    note="flag parsing",
)
add("head_noeol", "pipe", "head", "-n 2", P(NOEOL))
add("head_empty", "pipe", "head", "", P(EMPTY))
add("head_ctrl", "pipe", "head", "", P(CTRL))
add("head_ff", "pipe", "head", "-n 1", P(FF))
add("head_utf8", "pipe", "head", "-n 2", P(UTF8))

# ---------------------------------------------------------------- cat (pipe)
add("cat_plain", "pipe", "cat", "", P(PLAIN))
add("cat_ctrl", "pipe", "cat", "", P(CTRL), note="identity passthrough")
add("cat_nul", "pipe", "cat", "", P(NUL))
add("cat_v", "pipe", "cat", "-v", P(CTRL))
add("cat_A", "pipe", "cat", "-A", P(TABS))
add("cat_n", "pipe", "cat", "-n", P(PLAIN))
add("cat_b", "pipe", "cat", "-b", P(BLANKS))
add("cat_s", "pipe", "cat", "-s", P(BLANKS))
add("cat_E", "pipe", "cat", "-E", P(NOEOL))
add("cat_T", "pipe", "cat", "-T", P(TABS))
add("cat_e", "pipe", "cat", "-e", P(CTRL))
add("cat_noeol", "pipe", "cat", "", P(NOEOL))
add("cat_utf8", "pipe", "cat", "", P(UTF8))
add("cat_empty", "pipe", "cat", "", P(EMPTY))
add("cat_crlf", "pipe", "cat", "", P(CRLF))
add("cat_file", "pipe", "cat", "{PLAINFILE}", P(b"ignored\n"), note="file operand",
    require_empty_stderr=True)
add("cat_file_ctrl", "pipe", "cat", "{ESCFILE}", P(b"ignored\n"),
    note="file operand, carried bytes", require_empty_stderr=True)
add("cat_file_n", "pipe", "cat", "-n {PLAINFILE}", P(b"ignored\n"),
    note="file operand + numbering", require_empty_stderr=True)

# ---------------------------------------------------------------- sed (pipe)
add("sed_s", "pipe", "sed", "s/a/A/", P(PLAIN))
add("sed_sg", "pipe", "sed", "s/a/A/g", P(PLAIN))
add("sed_1d", "pipe", "sed", "1d", P(PLAIN))
add("sed_2d", "pipe", "sed", "2d", P(PLAIN))
add("sed_e", "pipe", "sed", "-e s/a/A/", P(PLAIN))
add("sed_noeol", "pipe", "sed", "s/a/A/", P(NOEOL))
add("sed_crlf", "pipe", "sed", "s/a/A/", P(CRLF))
add("sed_ctrl", "pipe", "sed", "s/a/A/", P(CTRL))
add("sed_utf8", "pipe", "sed", "s/a/A/", P(UTF8))
add("sed_dot", "pipe", "sed", "s/x./Y/", P(DOT), note="BRE dot")
add("sed_class", "pipe", "sed", "s/[abc]/Z/g", P(PLAIN), note="BRE bracket")
add("sed_empty", "pipe", "sed", "s/a/A/", P(EMPTY))
add("sed_ff", "pipe", "sed", "s/a/A/", P(FF))

# ---------------------------------------------------------------- grep (pipe)
add("grep_basic", "pipe", "grep", "beta", P(PLAIN))
add("grep_c", "pipe", "grep", "-c a", P(PLAIN))
add("grep_n", "pipe", "grep", "-n beta", P(PLAIN))
add("grep_i", "pipe", "grep", "-i ALPHA", P(PLAIN))
add("grep_v", "pipe", "grep", "-v alpha", P(PLAIN))
add("grep_q", "pipe", "grep", "-q beta", P(PLAIN), note="quiet, rc 0")
add("grep_qmiss", "pipe", "grep", "-q zzz", P(PLAIN), note="quiet miss, rc 1")
add("grep_cn", "pipe", "grep", "-cn a", P(PLAIN), note="combined flags")
add("grep_anchor", "pipe", "grep", "^al", P(PLAIN))
add("grep_noeol", "pipe", "grep", "gamma", P(NOEOL))
add("grep_noeol_nomatch", "pipe", "grep", "zzz", P(NOEOL), note="rc 1")
add("grep_empty", "pipe", "grep", "a", P(EMPTY))
add("grep_ctrl", "pipe", "grep", "a", P(CTRL))
add("grep_utf8", "pipe", "grep", "é", P(UTF8))
# The non-ASCII fold is locale- and build-dependent: GNU grep folds \xc3\xa9
# only in a UTF-8 locale, WinuxCmd grep does not fold it at all, and the inline
# arm uses Rust to_lowercase() (locale-independent).  Pin LC_ALL=C for all three
# legs so the console locale cannot flip the verdict: under C none of the
# external greps fold, so a build whose inline arm folds Unicode (ignoring the
# locale) is a real, detectable divergence rather than console noise.
add(
    "grep_utf8_i",
    "pipe",
    "grep",
    "-i É",
    P(UTF8),
    note="non-ASCII fold, LC_ALL=C pinned; external greps do not fold, inline must not either",
    env={"LC_ALL": "C", "LANG": "C"},
)
add("grep_empty_pattern", "pipe", "grep", "''", P(PLAIN))
add("grep_dot", "pipe", "grep", "x.", P(DOT), note="regex -> external fallback")
add("grep_v_all", "pipe", "grep", "-v a", P(PLAIN), note="all match -> rc 1")

# ---------------------------------------------------------------- wc (pipe)
add("wc_c", "pipe", "wc", "-c", P(PLAIN))
add(
    "wc_c_ctrl",
    "pipe",
    "wc",
    "-c",
    P(CTRL),
    known="counts carrier-encoded transport text (rubash#383)",
    note="known byte-count class",
)
add("wc_l", "pipe", "wc", "-l", P(PLAIN))
add("wc_l_ctrl", "pipe", "wc", "-l", P(CTRL))
add("wc_w", "pipe", "wc", "-w", P(PLAIN))
add(
    "wc_w_ff",
    "pipe",
    "wc",
    "-w",
    P(FF),
    known="form feed carried as U+E000 U+E00C is no longer whitespace",
    note="payload decoding / counting unit",
)
add("wc_default", "pipe", "wc", "", P(PLAIN))
add(
    "wc_default_ctrl",
    "pipe",
    "wc",
    "",
    P(CTRL),
    known="bytes column counts carrier-encoded transport text",
    note="known byte-count class",
)
add(
    "wc_default_ff",
    "pipe",
    "wc",
    "",
    P(FF),
    known="words column and bytes column diverge on carried whitespace",
    note="payload decoding / counting unit",
)
add("wc_noeol", "pipe", "wc", "-c", P(NOEOL))
add("wc_c_nul", "pipe", "wc", "-c", P(NUL))
add(
    "wc_c_ff",
    "pipe",
    "wc",
    "-c",
    P(b"a\x0cb\n"),
    known="carried form feed expands 1 byte to 6",
    note="payload decoding / byte count",
)
add(
    "wc_c_producer_echo",
    "pipe",
    "wc",
    "-c",
    "echo -e '\\033[31m'",
    known="builtin producer's control byte carried into wc -c",
    note="known byte-count class",
)
add(
    "wc_c_producer_printf",
    "pipe",
    "wc",
    "-c",
    "printf '\\033[31m'",
    known="builtin producer's control byte carried into wc -c",
    note="known byte-count class",
)
add("wc_utf8_c", "pipe", "wc", "-c", P(UTF8))
add("wc_utf8_w", "pipe", "wc", "-w", P(UTF8))
add("wc_empty", "pipe", "wc", "", P(EMPTY))
add("wc_m", "pipe", "wc", "-m", P(UTF8), note="chars -> external fallback")
add("wc_lc", "pipe", "wc", "-lc", P(PLAIN), note="combined -> external fallback")
add("wc_large", "pipe", "wc", "-c", large_producer(), note="truncation/re-encode")

# ---------------------------------------------------------------- tr (pipe)
add("tr_upper", "pipe", "tr", "a-z A-Z", P(PLAIN))
add("tr_simple", "pipe", "tr", "abc xyz", P(PLAIN))
add("tr_nl", "pipe", "tr", "'\\n' '_'", P(PLAIN), note="special \\n operand")
add("tr_class", "pipe", "tr", "'[:lower:]' '[:upper:]'", P(PLAIN))
add(
    "tr_class_cntrl",
    "pipe",
    "tr",
    "'[:cntrl:]' X",
    P(CTRL),
    known="rubash#707: carried control bytes are not members of [:cntrl:]",
    note="payload decoding",
)
add(
    "tr_class_space",
    "pipe",
    "tr",
    "'[:space:]' _",
    P(FF),
    known="rubash#707: carried form feed is not whitespace to the set matcher",
    note="payload decoding",
)
add(
    "tr_cntrl_mixed",
    "pipe",
    "tr",
    "'[:cntrl:]' X",
    P(b"a\x0b\x0cb\n"),
    known="rubash#707: 0x0b (not carried) maps, 0x0c (carried) escapes",
    note="payload decoding: only carrier-set bytes escape",
)
add("tr_d", "pipe", "tr", "-d a", P(PLAIN), note="option -> external fallback")
add("tr_utf8", "pipe", "tr", "a-z A-Z", P(UTF8), note="non-ascii set -> fallback")
add("tr_noeol", "pipe", "tr", "a-z A-Z", P(NOEOL))
add("tr_empty", "pipe", "tr", "a-z A-Z", P(EMPTY))
add("tr_ctrl", "pipe", "tr", "a-z A-Z", P(CTRL))

# ================================================================ substitution
add("sub_sort", "subst", "sort", "", P(PLAIN))
add("sub_sort_u", "subst", "sort", "-u", P(PLAIN))
add("sub_sort_noeol", "subst", "sort", "", P(NOEOL))
add("sub_sort_empty", "subst", "sort", "", P(EMPTY))
add(
    "sub_sort_crlf",
    "subst",
    "sort",
    "",
    P(CRLF),
    note="FAST==EXT drop CR, GNU keeps it: shared text-mode behaviour, not a fast-path divergence",
)
add("sub_sort_utf8", "subst", "sort", "", P(UTF8), note="locale/sort order")
add(
    "sub_sort_case",
    "subst",
    "sort",
    "",
    P(b"z\nA\nB\na\n"),
    note="locale collation vs byte order",
)
add(
    "sub_sort_blank_last",
    "subst",
    "sort",
    "",
    P(b"b\na\n\n"),
    note="trailing empty line ordering",
)
add("sub_sed", "subst", "sed", "s/a/A/", P(PLAIN))
add("sub_sed_1d", "subst", "sed", "1d", P(PLAIN))
add("sub_sed_ctrl", "subst", "sed", "s/a/A/", P(CTRL))
add("sub_tr", "subst", "tr", "a-z A-Z", P(PLAIN))
add("sub_tr_nl", "subst", "tr", "'\\n' '_'", P(PLAIN))
add("sub_head", "subst", "head", "-n 2", P(PLAIN))
add("sub_head_default", "subst", "head", "", P(PLAIN))
add(
    "sub_head_negn1",
    "subst",
    "head",
    "-n -1",
    P(PLAIN),
    known="rubash#708: same head_line_count default-10 defect inside $()",
    note="GNU -n -1 = all but last line",
)
add(
    "sub_head_badnum",
    "subst",
    "head",
    "-n abc",
    P(PLAIN),
    known="rubash#708: invalid number silently treated as default 10",
    note="flag parsing",
)
add("sub_head_noeol", "subst", "head", "-n 2", P(NOEOL))
add("sub_grep", "subst", "grep", "beta", P(PLAIN))
add("sub_grep_miss", "subst", "grep", "zzz", P(PLAIN), note="rc 1")
add("sub_grep_ctrl", "subst", "grep", "a", P(CTRL))
add("sub_wc_c", "subst", "wc", "-c", P(PLAIN))
add(
    "sub_wc_c_ctrl",
    "subst",
    "wc",
    "-c",
    P(CTRL),
    known="same byte-count class inside $()",
    note="known byte-count class",
)
add("sub_wc_l", "subst", "wc", "-l", P(PLAIN))
add("sub_wc_default", "subst", "wc", "", P(PLAIN), note="default is -l only")
add("sub_tail", "subst", "tail", "-n 2", P(PLAIN))
add("sub_tail_default", "subst", "tail", "", P(PLAIN))
add("sub_tail_badnum", "subst", "tail", "-n abc", P(PLAIN), note="falls back to external (values.rs tail arm dormant)")
add("sub_tail_noeol", "subst", "tail", "-n 2", P(NOEOL))
add("sub_tail_empty", "subst", "tail", "", P(EMPTY))
add("sub_uniq", "subst", "uniq", "", P(b"a\na\nb\nb\na\n"))
add("sub_uniq_plain", "subst", "uniq", "", P(PLAIN))
add("sub_uniq_noeol", "subst", "uniq", "", P(b"a\na"))

# ================================================================ documented in-process behaviours
# sleep (fast-path builtin: builtins/sleep.rs)
add("sleep_zero", "plain", "sleep", "0")
add("sleep_missing", "plain", "sleep", "", stderr=True)
add("sleep_invalid", "plain", "sleep", "abc", stderr=True)
add("sleep_badunit", "plain", "sleep", "1x", stderr=True)
add("sleep_unit_s", "plain", "sleep", "0s")
add("sleep_unit_h", "plain", "sleep", "0h")
add("sleep_dashdash", "plain", "sleep", "-- 0")

# dirname / basename (fast paths: executor/printf_path_builtins.rs)
add(
    "dirname_simple",
    "plain",
    "dirname",
    "/a/b/c",
    known="MSYS `/x` -> `X:` translation applied only on the fast path",
    note="path translation: FAST A:/b vs /a/b",
)
add("dirname_noslash", "plain", "dirname", "foo")
add("dirname_dot", "plain", "dirname", ".")
add("dirname_root", "plain", "dirname", "/")
add(
    "dirname_trailing",
    "plain",
    "dirname",
    "/a/b/",
    known="MSYS translation: FAST A: vs /a",
    note="path translation",
)
add(
    "dirname_doubleslash",
    "plain",
    "dirname",
    "//",
    known="rubash#709: POSIX leading // not preserved",
    note="FAST / vs //",
)
add(
    "dirname_tripleslash",
    "plain",
    "dirname",
    "///",
    note="FAST and GNU ('/') agree; only winuxcmd EXT is wrong (//)",
)
add(
    "dirname_lead_double_letter",
    "plain",
    "dirname",
    "//a",
    known="rubash#709: POSIX leading // not preserved",
    note="FAST / vs //",
)
add(
    "dirname_letter_double",
    "plain",
    "dirname",
    "/a//b",
    known="MSYS translation + internal // collapse",
    note="path translation",
)
add("dirname_dotdot", "plain", "dirname", "../a")
add("dirname_multi", "plain", "dirname", "a/b c/d")
add("dirname_dash", "plain", "dirname", "-a")
add("dirname_dashdash", "plain", "dirname", "-- -a")
add(
    "dirname_empty",
    "plain",
    "dirname",
    "''",
    known="rubash#709: empty operand must yield '.'",
    note="FAST / vs .",
)
add(
    "basename_empty",
    "plain",
    "basename",
    "''",
    known="rubash#709: empty operand must yield empty string",
    note="FAST / vs empty",
)
add("basename_simple", "plain", "basename", "/a/b/c.txt")
add("basename_suffix", "plain", "basename", "/a/b.txt .txt")
add("basename_suffix_partial", "plain", "basename", "b.txt txt")
add("basename_root", "plain", "basename", "/")
add("basename_trailing", "plain", "basename", "/a/b/")
add("basename_noslash", "plain", "basename", "foo")
add(
    "basename_doubleslash",
    "plain",
    "basename",
    "//",
    note="FAST==EXT ('/'), GNU ('//'): shared POSIX divergence",
)
add("basename_dash", "plain", "basename", "-a")
add("basename_extra", "plain", "basename", "a b c", stderr=True)

# uname / arch (identity tools, no external fallback)
add(
    "uname_s",
    "plain",
    "uname",
    "-s",
    known="identity persona: rubash reports MSYS_NT, winuxcmd Windows_NT, GNU MINGW64_NT (by design, rubash#154)",
    note="identity tool, intentional divergence",
)
add(
    "uname_a",
    "plain",
    "uname",
    "-a",
    known="identity persona fields intentionally differ (rubash#154)",
    note="identity tool, intentional divergence",
)
add(
    "arch_plain",
    "plain",
    "arch",
    "",
    note="rubash persona x86_64 matches winuxcmd/GNU on this host",
)

# env (in-process coreutils semantics, no fallback)
add("env_print", "plain", "env", "", note="sorted env dump, informational", informational=True)
add("env_set", "plain", "env", "FOO=bar", informational=True)


def build_body(case, cmd):
    kind = case["kind"]
    prog = case["prog"]
    args = resolve_placeholders(case["args"])
    producer = resolve_placeholders(case["producer"])
    if kind == "plain":
        # `cmd` already includes the external form for the program name.
        return (cmd + " " + args).strip()
    if kind == "pipe":
        return "%s | %s %s" % (producer, cmd, args)
    if kind == "subst":
        return "x=$(%s | %s %s); rc=$?; printf '%%s' \"$x\"; exit $rc" % (
            producer,
            cmd,
            args,
        )
    raise ValueError(kind)


def run(shell, body, extra_env=None):
    if shell == "gnu":
        argv = [GNU_BASH, "-c", body]
        run_env = dict(os.environ)
        run_env["PATH"] = GNU_PATH + ";" + run_env.get("PATH", "")
    else:
        argv = [NIU, "-c", body]
        run_env = dict(os.environ)
    if extra_env:
        run_env.update(extra_env)
    try:
        p = subprocess.run(
            argv, capture_output=True, timeout=120, env=run_env
        )
        return p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired:
        return b"<timeout>", b"", -1
    except FileNotFoundError:
        return b"", ("%s: not found" % argv[0]).encode(), 127


def main():
    global _LARGE_FILE, _FIXTURES, EXT_DISABLED
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--only", action="append", default=[], help="run only matching id substrings")
    ap.add_argument("--list", action="store_true", help="list case ids and exit")
    ap.add_argument("--json", help="write full results as JSON to this path")
    ap.add_argument("--show-ok", action="store_true", help="print passing cases too")
    ap.add_argument(
        "--no-ext",
        action="store_true",
        help="run without the WinuxCmd reference (every case compared against GNU)",
    )
    args = ap.parse_args()

    if args.list:
        for c in CASES:
            print(c["id"])
        return 0

    # Toolchain preflight: fail loudly rather than degrade.  A missing WX_BIN
    # used to silently downgrade the reference to GNU, misattributing WinuxCmd
    # gaps to rubash (e.g. wc_m).
    def fatal(message):
        print("fast-path-differential: " + message, file=sys.stderr)
        return 2

    if not os.path.isfile(NIU):
        return fatal("NIU not found: %s (set NIU=...)" % NIU)
    if not os.path.isfile(GNU_BASH):
        # run() reports a clean "not found" for a missing shell, but the GNU
        # reference is required for classification; warn, do not fail.
        print("fast-path-differential: warning: GNU_BASH not found: %s" % GNU_BASH, file=sys.stderr)
    EXT_DISABLED = args.no_ext
    if not args.no_ext and not os.path.isdir(WX_BIN):
        return fatal(
            "WX_BIN not found: %s (set WX_BIN=... or pass --no-ext)" % WX_BIN
        )

    os.makedirs(TMPDIR, exist_ok=True)
    # Per-process scratch directory: two concurrent harness instances never see
    # each other's fixtures, so a fixturing error cannot masquerade as
    # FAST/EXT agreement (the B1 false-PASS mode).
    scratch = tempfile.mkdtemp(prefix="fpd-%d-" % os.getpid(), dir=TMPDIR)
    _LARGE_FILE = os.path.join(scratch, "large.txt")
    plain_fixture = os.path.join(scratch, "plain.txt")
    esc_fixture = os.path.join(scratch, "esc.txt")
    _FIXTURES = {"PLAINFILE": plain_fixture, "ESCFILE": esc_fixture}

    try:
        with open(_LARGE_FILE, "wb") as fh:
            fh.write(b"y" * 200000 + b"\n")
        with open(plain_fixture, "wb") as fh:
            fh.write(PLAIN)
        with open(esc_fixture, "wb") as fh:
            fh.write(CTRL)
        # Integrity gate: a truncated/missing fixture must fail, not silently
        # compare empty stdout on all three legs.
        for path, expected in ((plain_fixture, PLAIN), (esc_fixture, CTRL)):
            with open(path, "rb") as fh:
                if fh.read() != expected:
                    return fatal("fixture content mismatch: %s" % path)

        selected = [
            c for c in CASES if not args.only or any(s in c["id"] for s in args.only)
        ]
        if not selected:
            print(
                "fast-path-differential: --only %r matched no cases"
                % (args.only,),
                file=sys.stderr,
            )
            return 1

        ext_ref = "DISABLED (--no-ext)" if EXT_DISABLED else WX_BIN
        print("FAST=%s  EXT=%s  GNU=%s" % (NIU, ext_ref, GNU_BASH))

        return run_matrix(args, selected)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def run_matrix(args, selected):
    results = []
    regressions = []
    quietly_fixed = []
    for case in selected:
        cid = case["id"]
        body_fast = build_body(case, case["prog"])
        ext = ext_cmd(case["prog"])
        body_ext = build_body(case, ext) if ext else None
        body_gnu = build_body(case, case["prog"])

        f = run("niu", body_fast, case["env"])
        e = run("niu", body_ext, case["env"]) if body_ext else None
        g = run("gnu", body_gnu, case["env"])

        if case["informational"]:
            results.append(
                {
                    "id": cid,
                    "status": "INFO",
                    "known": case["known"],
                    "note": case["note"],
                    "stderr": case["stderr"],
                    "require_empty_stderr": case["require_empty_stderr"],
                    "fast": {"out": f[0].decode("latin-1"), "err": f[1].decode("latin-1"), "rc": f[2]},
                    "ext": (
                        {"out": e[0].decode("latin-1"), "err": e[1].decode("latin-1"), "rc": e[2]}
                        if e
                        else None
                    ),
                    "gnu": {"out": g[0].decode("latin-1"), "err": g[1].decode("latin-1"), "rc": g[2]},
                }
            )
            if args.show_ok:
                print("[INFO   ] %-24s %s" % (cid, case["note"]))
            continue

        # Primary signal: the fast path must reproduce its own external
        # fallback.  When there is no external counterpart (`:`, `trap`) the
        # GNU bash builtin is the reference.
        ref = e if e else g
        ref_name = "EXT" if e else "GNU"
        match = (f[0] == ref[0]) and (f[2] == ref[2])
        if case["stderr"]:
            match = match and (f[1] == ref[1])
        if case["require_empty_stderr"]:
            # Fixture-backed cases: a missing/unreadable operand must never be
            # recorded as agreement, even if all legs agree on the error.
            match = match and (f[1] == b"") and (ref[1] == b"")
        match_gnu = (f[0] == g[0]) and (f[2] == g[2])
        ext_gnu = ((e[0] == g[0]) and (e[2] == g[2])) if e else None

        status = "PASS" if match else "DIVERGE"
        if not match:
            # Distinguish a fast path that deviates from a correct fallback from
            # a fallback that is itself wrong (a winuxcmd defect): only the
            # former is a rubash fast-path divergence.
            if match_gnu and ext_gnu is False:
                status = "EXTBUG"
        if case["known"]:
            if match:
                quietly_fixed.append(cid)
                status = "FIXED?"
            else:
                status = "KNOWN"
        elif not match and status == "DIVERGE":
            regressions.append(cid)

        results.append(
            {
                "id": cid,
                "status": status,
                "reference": ref_name,
                "known": case["known"],
                "note": case["note"],
                "stderr": case["stderr"],
                "require_empty_stderr": case["require_empty_stderr"],
                "match_external": match,
                "match_gnu": match_gnu,
                "external_matches_gnu": ext_gnu,
                "fast": {"out": f[0].decode("latin-1"), "err": f[1].decode("latin-1"), "rc": f[2]},
                "ext": (
                    {"out": e[0].decode("latin-1"), "err": e[1].decode("latin-1"), "rc": e[2]}
                    if e
                    else None
                ),
                "gnu": {"out": g[0].decode("latin-1"), "err": g[1].decode("latin-1"), "rc": g[2]},
            }
        )

        if status == "PASS" and not args.show_ok:
            continue
        print("[%-7s] %-24s %s" % (status, cid, case["note"]))
        if status in ("DIVERGE", "FIXED?", "EXTBUG"):
            print("    FAST  out=%r rc=%s" % (f[0][:120], f[2]))
            print("    %-4s  out=%r rc=%s" % (ref_name, ref[0][:120], ref[2]))
            if e:
                print("    GNU   out=%r rc=%s" % (g[0][:120], g[2]))

    print()
    print(
        "cases=%d pass=%d known=%d extbugs=%d info=%d regressions=%d fixed=%d"
        % (
            len(results),
            sum(1 for r in results if r["status"] == "PASS"),
            sum(1 for r in results if r["status"] == "KNOWN"),
            sum(1 for r in results if r["status"] == "EXTBUG"),
            sum(1 for r in results if r["status"] == "INFO"),
            len(regressions),
            len(quietly_fixed),
        )
    )
    if regressions:
        print("REGRESSIONS: " + ", ".join(regressions))
    if quietly_fixed:
        print("KNOWN CASES NOW MATCHING (update expectations): " + ", ".join(quietly_fixed))

    if args.json:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump(results, fh, indent=2)
        print("wrote " + args.json)

    return 1 if (regressions or quietly_fixed) else 0


if __name__ == "__main__":
    sys.exit(main())
