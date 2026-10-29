#!/usr/bin/env python3
"""Live-ConPTY execution-level driver for the stty builtin dump/resize faces.

Spawns the engine under a real ConPTY (pywinpty) and drives the
interactive REPL the way a TUI script does. The non-terminal pins in
tests/issue282_stty_console_regressions.rs can only prove diagnostics —
these legs prove the faces that need a real console:

  stty-a-dump   `stty -a` prints the owner-approved 7-line WinuxCmd dump
                shape (GNU-default control chars, min/time from
                PROCESSED_INPUT, speed line carrying REAL CONOUT$
                geometry, console-mode-driven flag lines, `sane -icanon`
                summary).
  geometry-matches
                the rows/columns on the -a speed line equal the
                `stty size` pair (one geometry source).
  rows-cols-set `stty rows R cols C` resizes so a later `stty size`
                reflects it (GNU stty.c:1216-1234 set_window_size), and
                the bare `rows`/`cols` queries follow.
  mixed-order   `stty sane rows R cols C` applies settings left to right
                (GNU main() settings pass), and an output style with a
                setting is refused ON A TTY too (stty.c:1385).
  invalid-integer
                `stty rows abc` on a TTY reaches the apply pass and
                reports GNU's `invalid integer argument: 'abc'`
                (stty.c:1220 integer_arg) — the non-terminal pins can
                only show the checking pass deferring it.
  gfmt-hex      `stty -g` prints one lowercase-hex field (the honest
                console-mode DWORD, NOT GNU termios gfmt), and `-g` with
                a setting is refused.

Every rc marker uses the `echo "X#""RC=$?"` split-string shape so the
echoed command line can never satisfy a marker check. Exit code 0 = every
leg passed; 1 = a leg failed; 2 = pywinpty missing (WSL/Linux CI — the
live-console face needs a real console host).

Usage: python conpty_stty_probe.py <rubash.exe> [leg ...]
Prints one `leg=PASS` / `leg=FAIL detail` line per requested leg.
"""

import os
import re
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

# 200 columns: no dump line wraps, so exact-line assertions hold.
DIMENSIONS = (50, 200)

A_LINE1 = (
    "intr = ^C; quit = ^\\; erase = ^?; kill = ^U; eof = ^D; eol = <undef>; "
    "eol2 = <undef>; swtch = <undef>; start = ^Q; stop = ^S; susp = ^Z; "
    "rprnt = ^R; werase = ^W; lnext = ^V; discard = ^O;"
)
A_LINE4 = "-ignbrk -brkint -parmrk -istrip -inlcr -igncr icrnl -ixon -ixoff -iuclc -ixany -imaxbel -iutf8"
A_LINE5 = "-opost -olcuc -ocrnl -onlcr -onocr -onlret -ofill -ofdel nl0 cr0 tab0 bs0 vt0 ff0"
A_LINE6 = "echo -echoe echok -echonl -noflsh -tostop echoctl -echoprt echoke -flusho -extproc"
GEOMETRY = re.compile(r"speed 38400 baud; rows (\d+); columns (\d+); line = 0;")
SIZE_PAIR = re.compile(r"(\d+) (\d+)")


class Session:
    """One interactive engine REPL under ConPTY with an output pump."""

    def __init__(self, exe):
        self.proc = PtyProcess.spawn(
            [exe, "--norc", "--noprofile"],
            cwd=tempfile.gettempdir(),
            dimensions=DIMENSIONS,
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
        """Wait for an output line carrying `needle` (callers use the
        split-string marker shape so the echoed command never matches)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            for line in self.text().split("\n"):
                if needle in line:
                    return line.strip()
            time.sleep(0.03)
        return None

    def wait_for(self, needle, timeout=12.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if needle in self.text():
                return
            time.sleep(0.05)
        raise TimeoutError(f"{needle!r} never arrived; tail={self.text()[-300:]!r}")

    def close(self):
        try:
            self.proc.terminate(force=True)
        except Exception:
            pass


def collect_dump_lines(s):
    """The stripped stty -a output lines. Readline prompt/echo lines carry
    the `5.3#` prompt and are skipped wholesale; dump lines are recognized
    by their first (stripped) token, which no prompt line starts with."""
    out = []
    for line in s.text().split("\n"):
        if "5.3#" in line:
            continue
        stripped = line.strip()
        if stripped.startswith(
            (
                "intr = ",
                "min = ",
                "speed ",
                "-ignbrk",
                "ignbrk",
                "-opost",
                "opost",
                "echo ",
                "-echo ",
                "sane ",
            )
        ):
            out.append(stripped)
    return out


def leg_stty_a_dump(s):
    s.write("stty -a\r")
    s.wait_for("sane -icanon")
    dump = collect_dump_lines(s)
    assert len(dump) >= 7, f"expected 7 dump lines, got {len(dump)}: tail={s.text()[-400:]!r}"
    assert dump[0] == A_LINE1, f"control-char line: {dump[0]!r}"
    assert dump[1] == "min = 1; time = 0;", f"min line: {dump[1]!r}"
    geo = GEOMETRY.fullmatch(dump[2])
    assert geo, f"speed line: {dump[2]!r}"
    assert dump[3] == A_LINE4, f"input line: {dump[3]!r}"
    assert dump[4] == A_LINE5, f"output line: {dump[4]!r}"
    assert dump[5] == A_LINE6, f"local line: {dump[5]!r}"
    assert dump[6] == "sane -icanon", f"summary line: {dump[6]!r}"


def last_size_pair(s):
    pair = None
    for line in s.text().split("\n"):
        m = SIZE_PAIR.fullmatch(line.strip())
        if m:
            pair = (m.group(1), m.group(2))
    return pair


def leg_geometry_matches_size(s):
    s.write('stty -a; stty size; echo "GM#""RC=$?"\r')
    line = s.marker("GM#RC=")
    assert line and "RC=0" in line, f"marker={line!r}"
    geo = None
    for text_line in s.text().split("\n"):
        m = GEOMETRY.fullmatch(text_line.strip())
        if m:
            geo = m
    assert geo, f"no speed line in output: tail={s.text()[-400:]!r}"
    size = last_size_pair(s)
    assert size, f"no size line in output: tail={s.text()[-400:]!r}"
    assert size == (geo.group(1), geo.group(2)), (
        f"-a says {geo.group(1)} {geo.group(2)}, size says {size!r}"
    )


def leg_rows_cols_set(s):
    s.write('stty rows 20 cols 60; echo "RS#""RC=$?"\r')
    line = s.marker("RS#RC=")
    assert line and "RC=0" in line, f"resize rc marker={line!r}"
    s.write('stty size; echo "RSQ#""RC=$?"\r')
    line = s.marker("RSQ#RC=")
    assert line and "RC=0" in line, f"query rc marker={line!r}"
    size = last_size_pair(s)
    assert size == ("20", "60"), f"stty size after resize: {size!r}"


def leg_bare_dimension_queries(s):
    s.write('stty rows 20 cols 60; stty rows; stty cols; echo "BR#""RC=$?"\r')
    line = s.marker("BR#RC=")
    assert line and "RC=0" in line, f"marker={line!r}"
    digits = [
        line.strip()
        for line in s.text().split("\n")
        if re.fullmatch(r"\d+", line.strip())
    ]
    assert "20" in digits and "60" in digits, f"bare queries produced {digits!r}"


def leg_mixed_order(s):
    s.write('stty sane rows 25 cols 80; echo "MO#""RC=$?"\r')
    line = s.marker("MO#RC=")
    assert line and "RC=0" in line, f"marker={line!r}"
    s.write('stty size; echo "MOQ#""RC=$?"\r')
    line = s.marker("MOQ#RC=")
    assert line and "RC=0" in line, f"marker={line!r}"
    size = last_size_pair(s)
    assert size == ("25", "80"), f"stty size after sane+resize: {size!r}"
    s.write('stty -a sane; echo "MOS#""RC=$?"\r')
    line = s.marker("MOS#RC=")
    assert line and "RC=1" in line, f"styles-on-tty marker={line!r}"


def leg_invalid_integer(s):
    s.write('stty rows abc; echo "II#""RC=$?"\r')
    line = s.marker("II#RC=")
    assert line and "RC=1" in line, f"marker={line!r}"
    assert "stty: invalid integer argument: 'abc'" in s.text(), (
        f"diagnostic missing; tail={s.text()[-300:]!r}"
    )


def leg_gfmt_hex(s):
    s.write('stty -g; echo "GH#""RC=$?"\r')
    line = s.marker("GH#RC=")
    assert line and "RC=0" in line, f"marker={line!r}"
    gfmt = None
    for text_line in s.text().split("\n"):
        stripped = text_line.strip()
        if re.fullmatch(r"[0-9a-f]+", stripped) and stripped not in ("0", "1"):
            gfmt = stripped
    assert gfmt, f"no lowercase-hex -g line; tail={s.text()[-300:]!r}"
    s.write('stty -g rows 40; echo "GHS#""RC=$?"\r')
    line = s.marker("GHS#RC=")
    assert line and "RC=1" in line, f"styles marker={line!r}"


LEGS = {
    "stty-a-dump": leg_stty_a_dump,
    "geometry-matches": leg_geometry_matches_size,
    "rows-cols-set": leg_rows_cols_set,
    "bare-dimension-queries": leg_bare_dimension_queries,
    "mixed-order": leg_mixed_order,
    "invalid-integer": leg_invalid_integer,
    "gfmt-hex": leg_gfmt_hex,
}


def main():
    if len(sys.argv) < 2:
        print("usage: conpty_stty_probe.py <rubash.exe> [leg ...]")
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
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
