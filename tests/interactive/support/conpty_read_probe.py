#!/usr/bin/env python3
"""Live-ConPTY execution-level driver for the rubash#806 read regressions.

Spawns the engine under a real ConPTY (pywinpty), drives the interactive
REPL keystroke by keystroke, and asserts marker payloads — the same shape
the #805 triage probes and scripts/smoke-read-tty-pty.py use. One leg per
ticket symptom face:

  n1-typed-ahead      #806-1  `read -n1` with the single key already queued
                              when read starts (one write burst) completes
                              with that key (GNU: one keypress, read.def
                              ttfd_onechar, no Enter).
  n1-while-waiting    #806-1  control: key arriving while read is blocked.
  n1-enter-icrnl      #806-1  `read -n1` with Enter as the key: shtty.c:115
                              tt_setonechar forces ICRNL, so the CR is the
                              delimiter — zero chars, rc 0.
  t-key-in-window     #806-1  `read -t 2 -n1` with the key arriving inside
                              the deadline window while the submitting
                              Enter's key-release record sits at the queue
                              head (GNU select fires; the read takes it).
  t-line-complete     #806-2  plain `read -t 3 a`, whole line + Enter inside
                              the window: rc 0 with the FULL line (never the
                              cooked-ahead first byte).
  line-atomic         #806-2  after that read, the NEXT read must see only
                              fresh input — no cooked-ahead orphan.
  plain-queued        #806-2  plain `read a` with the payload line + its
                              Enter queued in the same burst: consumed with
                              that ONE Enter.
  plain-queued-cooked #806-2  payload queued while a prior `sleep 1` runs
                              (console in cooked mode): consumed, one Enter.
  t-partial-assign    contract `read -t 2 a` with a partial line and no
                              Enter: deadline assigns the partial record and
                              returns 128+SIGALRM (read.def:553).
  t-no-input          contract `read -t 1 -n1` with no input: deadline fires
                              (rc 142) — the niubash#280 protection.
  ctrld-read-eof      niubash#684  plain `read x` + Ctrl-D + Enter: the
                              0x04 is the EOF — rc 1, empty assignment,
                              and the shell survives for the next command
                              (GNU: kernel VEOF -> zread 0 -> read.def:767
                              eof=1, :949 failure). Pre-fix the byte was
                              plain data (or the EOF fell through to a
                              second blocking reader): the read hung for
                              the NEXT line and consumed it as the record.
  ctrld-midline-flush niubash#684  `abc` Ctrl-D `def` + Enter in one
                              line: VEOF flush — the marker is not data,
                              the record is "abcdef", rc 0.

Exit code 0 = every leg passed; 1 = a leg failed; 2 = pywinpty missing
(WSL/Linux CI — the live-console face needs a real console host).

Usage: python conpty_read_probe.py <rubash.exe> [leg ...]
Prints one `leg=PASS` / `leg=FAIL detail` line per requested leg.
"""

import os
import re
import subprocess
import sys
import tempfile
import threading
import time

ANSI = re.compile(
    r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[=>]"
)

try:
    from winpty import PtyProcess  # noqa: F401
except ImportError:
    print("SKIP pywinpty missing")
    sys.exit(2)


class Session:
    """One interactive engine REPL under ConPTY with an output pump."""

    def __init__(self, exe):
        self.proc = PtyProcess.spawn(
            [exe, "--norc", "--noprofile"],
            cwd=tempfile.gettempdir(),
            dimensions=(30, 100),
        )
        self.chunks = []
        self.pump = threading.Thread(target=self._pump, daemon=True)
        self.pump.start()

    def _pump(self):
        while self.proc.isalive():
            try:
                data = self.proc.read()
            except Exception:
                break
            if data:
                self.chunks.append(data)

    def text(self):
        return ANSI.sub("", "".join(self.chunks))

    def write(self, keys):
        self.proc.write(keys)

    def ready(self, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if "5.3#" in self.text():
                return
            time.sleep(0.05)
        raise TimeoutError(f"no prompt; tail={self.text()[-200:]!r}")

    def marker(self, needle, timeout=12.0):
        """Wait for an output line carrying `needle` that is not the echoed
        command line (the echo always contains 'echo')."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            for line in self.text().split("\n"):
                if needle in line and "echo" not in line:
                    return line.strip()
            time.sleep(0.03)
        return None

    def close(self):
        try:
            self.proc.terminate(force=True)
        except Exception:
            pass


def leg_n1_typed_ahead(s):
    s.write('IFS= read -rn1 k; echo "N1T#""RC=$? K=[$k]"\rq')
    line = s.marker("N1T#RC=")
    assert line and "RC=0" in line and "K=[q]" in line, f"marker={line!r}"


def leg_n1_while_waiting(s):
    s.write('IFS= read -rn1 k; echo "N1W#""RC=$? K=[$k]"\r')
    time.sleep(0.5)
    s.write("q")
    line = s.marker("N1W#RC=")
    assert line and "RC=0" in line and "K=[q]" in line, f"marker={line!r}"


def leg_n1_enter_icrnl(s):
    # Enter as the key: ICRNL (shtty.c:115) makes it the delimiter — the
    # record is empty and rc 0.
    s.write('IFS= read -rn1 k; echo "N1E#""RC=$? LEN=${#k}"\r\r')
    line = s.marker("N1E#RC=")
    assert line and "RC=0" in line and "LEN=0" in line, f"marker={line!r}"


def leg_t_key_in_window(s):
    s.write('IFS= read -rn1 -t 2 k; echo "T1W#""RC=$? K=[$k]"\r')
    time.sleep(0.4)
    s.write("q")
    line = s.marker("T1W#RC=")
    assert line and "RC=0" in line and "K=[q]" in line, (
        f"marker={line!r} (head-of-line release starved the deadline gate)"
    )


def leg_t_line_complete(s):
    s.write('read -t 3 a; echo "TLC#""RC=$? A=[$a]"\r')
    time.sleep(0.4)
    s.write("hello\r")
    line = s.marker("TLC#RC=")
    assert line and "RC=0" in line and "A=[hello]" in line, (
        f"marker={line!r} (cooked-ahead byte split the line)"
    )
    return line


def leg_line_atomic(s):
    # The read after a deadline read must see ONLY fresh input: the
    # cooked-ahead leftover of the previous line must not leak into it.
    s.write('read -t 3 a; echo "LAT#""RC=$? A=[$a]"; read b; echo "LAA#""B=[$b]"\r')
    time.sleep(0.4)
    s.write("hello\r")
    line = s.marker("LAT#RC=")
    assert line and "RC=0" in line and "A=[hello]" in line, f"marker={line!r}"
    time.sleep(0.3)
    s.write("zzz\r")
    line = s.marker("LAA#B=")
    assert line and "B=[zzz]" in line, f"orphan reached the next read: {line!r}"


def leg_plain_queued(s):
    s.write('read a; echo "PLQ#""A=[$a]"\rhello\r')
    line = s.marker("PLQ#A=")
    assert line and "A=[hello]" in line, f"marker={line!r}"


def leg_plain_queued_cooked(s):
    s.write('sleep 1; read a; echo "PLQ#""A=[$a]"\r')
    time.sleep(0.4)
    s.write("hello\r")
    line = s.marker("PLQ#A=")
    assert line and "A=[hello]" in line, f"marker={line!r}"


def leg_t_partial_assign(s):
    s.write('read -t 2 a; echo "TPA#""RC=$? A=[$a]"\r')
    time.sleep(0.4)
    s.write("hello")
    line = s.marker("TPA#RC=")
    # read.def:553: the deadline assigns the partial record, rc 128+SIGALRM.
    # (The one-char path preserves every consumed keystroke in the record;
    # GNU's line discipline holds partial bytes, so its record is empty —
    # the assigned-partial contract is what the engine pins here.)
    #
    # NEXT-READER CONSEQUENCE of that divergence (not read.def:553
    # equivalence — oracle WSL 5.3.0: the bytes GNU's deadline read left in
    # the line discipline are handed to the NEXT reader once an Enter
    # completes the held-back line — its `read b` saw B=[hello]; rubash
    # consumed them into `a`, so the next reader only sees input typed after
    # the timeout, B=[zzz]). Keep both models in mind before "aligning"
    # either side; neither loses the bytes, they move to different owners.
    assert line and "RC=142" in line and "A=[hello]" in line, f"marker={line!r}"


def leg_t_no_input(s):
    s.write('IFS= read -rn1 -t 1 k; echo "TNI#""RC=$?"\r')
    line = s.marker("TNI#RC=")
    assert line and "RC=142" in line, f"marker={line!r}"


def leg_ctrld_read_eof(s):
    # niubash#684 (ConPTY face): Ctrl-D during a plain canonical `read`
    # is the EOF, not data. The Enter after it completes the console
    # line the 0x04 sits in; the read must stop AT the marker (rc 1,
    # empty assignment) without consuming any later line, and the
    # shell must take the next command (GNU: read.def:767/:949 — the
    # EOF ends only the current input).
    s.write('read x; echo "D0#""RC=$? X=[$x]"\r')
    time.sleep(0.5)
    s.write("\x04")
    s.write("\r")
    line = s.marker("D0#RC=")
    assert line and "RC=1" in line and "X=[]" in line, f"marker={line!r}"
    s.write("echo ZZALIVE684\r")
    line = s.marker("ZZALIVE684")
    assert line, "shell did not survive the read EOF"


def leg_ctrld_midline_flush(s):
    # niubash#684: Ctrl-D mid-line is the VEOF flush — the buffered
    # prefix and the rest of the line form ONE record ("abcdef"),
    # the marker byte itself is never data, rc 0.
    s.write('read y; echo "DM#""RC=$? Y=[$y]"\r')
    time.sleep(0.5)
    s.write("abc\x04def\r")
    line = s.marker("DM#RC=")
    assert line and "RC=0" in line and "Y=[abcdef]" in line, \
        f"marker={line!r}"


LEGS = {
    "n1-typed-ahead": leg_n1_typed_ahead,
    "n1-while-waiting": leg_n1_while_waiting,
    "n1-enter-icrnl": leg_n1_enter_icrnl,
    "t-key-in-window": leg_t_key_in_window,
    "t-line-complete": leg_t_line_complete,
    "line-atomic": leg_line_atomic,
    "plain-queued": leg_plain_queued,
    "plain-queued-cooked": leg_plain_queued_cooked,
    "t-partial-assign": leg_t_partial_assign,
    "t-no-input": leg_t_no_input,
    "ctrld-read-eof": leg_ctrld_read_eof,
    "ctrld-midline-flush": leg_ctrld_midline_flush,
}


def kill_conpty_hosts():
    """Opt-in cleanup: pywinpty's terminate() can leak the ConPTY host
    process. Set RUBASH_CONPTY_CLEANUP=1 to sweep it (a shared dev box may
    host other sessions' consoles, so this never runs by default)."""
    if os.environ.get("RUBASH_CONPTY_CLEANUP") != "1":
        return
    try:
        subprocess.run(
            ["taskkill", "/F", "/IM", "OpenConsole.exe"],
            capture_output=True,
            timeout=10,
        )
    except Exception:
        pass


def main():
    if len(sys.argv) < 2:
        print("usage: conpty_read_probe.py <rubash.exe> [leg ...]")
        return 2
    exe = os.path.abspath(sys.argv[1])
    if not os.path.isfile(exe):
        print(f"SKIP engine missing: {exe}")
        return 2
    wanted = sys.argv[2:] or list(LEGS)
    unknown = [leg for leg in wanted if leg not in LEGS]
    if unknown:
        print(f"FAIL unknown legs: {unknown}")
        return 1

    failed = []
    try:
        for leg in wanted:
            session = Session(exe)
            try:
                session.ready()
                LEGS[leg](session)
                print(f"{leg}=PASS", flush=True)
            except Exception as err:  # a leg failure must never kill the sweep
                failed.append(leg)
                detail = repr(err).splitlines()[0][:200]
                print(f"{leg}=FAIL {detail}", flush=True)
            finally:
                session.close()
                time.sleep(0.1)
    finally:
        kill_conpty_hosts()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
