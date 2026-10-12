#!/usr/bin/env python3
"""ConPTY probe: `read` on a real console — dup descriptors and -t deadlines.

niubash#279: `exec 8<&0` then `IFS= read -rsn1 k <&8` must read one
keystroke (POSIX dup semantics), not collapse to rc=1 with the key lost.

niubash#280: `IFS= read -rsn1 -t 0.05 k` must return within the deadline
(rc 142, read.def:553) with no input; fractional seconds are the ESC
disambiguation primitive fzf-style TUIs need. The console read also runs
in one-char mode (read.def:570-590 ttfd_onechar/cbreak), so a single key
completes the read without Enter.

Exit codes: 0 pass, 1 fail, 2 skip (python deps missing).

Usage: python scripts/smoke-read-tty-pty.py [rubash.exe]
       (default binary: target/debug/rubash.exe)
"""

import sys
import tempfile
import time
import threading
from pathlib import Path

try:
    import pyte
    from winpty import PtyProcess
except ImportError as err:  # pragma: no cover - environment guard
    print(f"SKIP: python deps missing ({err}); needs pywinpty + pyte")
    sys.exit(2)

REPO = Path(__file__).resolve().parent.parent
COLS, ROWS = 100, 30
PROMPT = "bash-5.3#"


class Session:
    def __init__(self, exe, name):
        self.proc = PtyProcess.spawn(
            [str(exe), "--norc", "--noprofile"],
            cwd=tempfile.gettempdir(),
            dimensions=(ROWS, COLS),
        )
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.name = name
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        while self.proc.isalive():
            try:
                data = self.proc.read()
            except Exception:
                break
            if data:
                self.stream.feed(data)

    def text(self):
        return "\n".join(self.screen.display)

    def wait_prompt(self, timeout=15):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if PROMPT in self.text():
                return
            time.sleep(0.05)
        raise TimeoutError(f"no prompt; screen:\n{self.text()}")

    def send(self, keys, gap=0.3):
        time.sleep(gap)
        self.proc.write(keys)

    def result(self, needle, timeout=5):
        """Wait for the emitted marker line and return its payload. The
        marker is assembled from a shell variable so the typed command echo
        can never match it."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            lines = [
                line
                for line in self.text().splitlines()
                if needle in line and "echo" not in line
            ]
            if lines:
                return lines[-1].split(needle)[-1].strip()
            time.sleep(0.05)
        raise TimeoutError(f"no output line {needle!r}; screen:\n{self.text()}")

    def close(self):
        try:
            self.proc.terminate(force=True)
        except Exception:
            pass


def leg_dup_fd0_read(exe):
    """#279: exec 8<&0; read -rsn1 <&8 reads one keypress (rc=0 len=1)."""
    s = Session(exe, "dup")
    try:
        s.wait_prompt()
        s.send("exec 8<&0\r")
        # Marker via $M: the emitted text (DUP#K RC=...) differs from the
        # echoed command line ("DUP#""K RC=...").
        s.send('IFS= read -rsn1 k <&8; echo "DUP#""K RC=$? LEN=${#k} K=[$k]"\r')
        time.sleep(0.3)
        s.proc.write("Q")
        out = s.result("DUP#K RC=")
        assert out == "0 LEN=1 K=[Q]", f"dup console read: {out!r}"
        return out
    finally:
        s.close()


def leg_fd0_direct_read(exe):
    """Control for #279: fd0 direct single-key read (rc=0 len=1)."""
    s = Session(exe, "direct")
    try:
        s.wait_prompt()
        s.send('IFS= read -rsn1 k; echo "DIR#K RC=$? LEN=${#k} K=[$k]"\r')
        time.sleep(0.3)
        s.proc.write("Z")
        out = s.result("DIR#K RC=")
        assert out == "0 LEN=1 K=[Z]", f"direct console read: {out!r}"
        return out
    finally:
        s.close()


def leg_fractional_timeout(exe):
    """#280: -t 0.05 with no input returns 142 promptly."""
    s = Session(exe, "t005")
    try:
        s.wait_prompt()
        started = time.time()
        s.send('IFS= read -rsn1 -t 0.05 k; echo "TMO#UT RC=$?"\r')
        out = s.result("TMO#UT RC=", timeout=5)
        elapsed = time.time() - started
        assert out == "142", f"fractional timeout status: {out!r}"
        assert elapsed < 4, f"fractional timeout took {elapsed:.2f}s"
        return f"{out} in {elapsed:.2f}s"
    finally:
        s.close()


def leg_integer_timeout(exe):
    """#280: integer -t 1 keeps working on the console (rc=142)."""
    s = Session(exe, "t1")
    try:
        s.wait_prompt()
        s.send('IFS= read -rsn1 -t 1 k; echo "TMO#INT RC=$?"\r')
        out = s.result("TMO#INT RC=", timeout=8)
        assert out == "142", f"integer timeout status: {out!r}"
        return out
    finally:
        s.close()


def leg_key_without_enter(exe):
    """One-char console mode: `read -n1` completes on one keypress, no
    Enter (read.def ttfd_onechar); the non-silent read echoes the key."""
    s = Session(exe, "onechar")
    try:
        s.wait_prompt()
        s.send('IFS= read -rn1 k; echo "ONE#CH RC=$? K=[$k]"\r')
        time.sleep(0.3)
        s.proc.write("q")
        out = s.result("ONE#CH RC=")
        assert out == "0 K=[q]", f"onechar read: {out!r}"
        return out
    finally:
        s.close()


def leg_silent_read_no_echo(exe):
    """#282 leg 2: -s silences the console echo - the typed key never lands
    on screen; the result line contains it exactly once (the output)."""
    s = Session(exe, "silent")
    try:
        s.wait_prompt()
        s.send('IFS= read -rsn1 k; echo "SIL#K RC=$? K=[$k]"; echo "MARK#ER done"\r')
        time.sleep(0.3)
        s.proc.write("@")
        out = s.result("SIL#K RC=")
        assert out == "0 K=[@]", f"silent read: {out!r}"
        s.result("MARK#ER done")
        occurrences = s.text().count("@")
        assert occurrences == 1, (
            f"read -s echoed the key: '@' appears {occurrences} times"
        )
        return out
    finally:
        s.close()


def console_mode_available(s):
    """Console-mode APIs need a real conhost console. A winpty-style bridge
    (pywinpty's legacy backend) serves stdin as a plain character device,
    where GetConsoleMode fails and stty honestly reports the GNU
    non-terminal diagnostic - the legs then SKIP (真机待验)."""
    s.wait_prompt()
    s.send('stty size\r')
    time.sleep(0.6)
    return "Inappropriate ioctl" not in s.text()


def leg_stty_size(exe):
    """#282 leg 3: stty size prints 'rows cols' from the console."""
    s = Session(exe, "size")
    try:
        if not console_mode_available(s):
            return "SKIP (no console-mode API under this pty; verify on a real console)"
        s.wait_prompt()
        s.send('stty size; echo "SIZE#RC=$?"\r')
        # the size line '30 100' precedes the marker line
        deadline = time.time() + 5
        while time.time() < deadline:
            for line in s.text().splitlines():
                parts = line.strip().split()
                if len(parts) == 2 and parts[0].isdigit() and parts[1].isdigit():
                    if "stty" not in line and "echo" not in line:
                        out = s.result("SIZE#RC=")
                        assert out == "0", f"stty size rc: {out!r}"
                        return line.strip()
            time.sleep(0.05)
        raise AssertionError(f"no size line; screen:\n{s.text()}")
    finally:
        s.close()


def leg_stty_raw_read(exe):
    """#282 leg 4: stty raw -echo persists; a plain read then returns per
    character with no Enter (raw cleared the line discipline), and stty
    sane restores the canonical session."""
    s = Session(exe, "raw")
    try:
        if not console_mode_available(s):
            return "SKIP (no console-mode API under this pty; verify on a real console)"
        s.wait_prompt()
        s.send("stty raw -echo\r")
        time.sleep(0.3)
        s.send('IFS= read k; echo "RAW#RD K=[$k]"; stty sane; echo "SA#NE=$?"\r', gap=0.3)
        time.sleep(0.3)
        s.proc.write("w")
        out = s.result("RAW#RD K=")
        assert out == "[w]", f"stty raw read: {out!r}"
        sane = s.result("SA#NE=")
        assert sane == "0", f"stty sane rc: {sane!r}"
        return f"{out} sane={sane}"
    finally:
        s.close()


def leg_line_read_stays_canonical(exe):
    """#280/#282 companion: a plain newline-terminated read keeps the
    canonical line discipline - the full line arrives after Enter."""
    s = Session(exe, "canonical")
    try:
        s.wait_prompt()
        s.send('IFS= read k; echo "LIN#RD K=[$k]"\r')
        time.sleep(0.3)
        s.proc.write("hello")
        time.sleep(0.3)
        # not returned yet: the line needs Enter
        raised = False
        try:
            s.result("LIN#RD K=", timeout=1)
        except TimeoutError:
            raised = True
        assert raised, "plain read returned before Enter (canonical lost)"
        s.proc.write("\r")
        out = s.result("LIN#RD K=")
        assert out == "[hello]", f"canonical read: {out!r}"
        return out
    finally:
        s.close()


def main():
    exe = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target/debug/rubash.exe"
    legs = [
        ("dup-fd0-read", leg_dup_fd0_read),
        ("fd0-direct-read", leg_fd0_direct_read),
        ("fractional-timeout", leg_fractional_timeout),
        ("integer-timeout", leg_integer_timeout),
        ("key-without-enter", leg_key_without_enter),
        ("silent-read-no-echo", leg_silent_read_no_echo),
        ("stty-size", leg_stty_size),
        ("stty-raw-read", leg_stty_raw_read),
        ("line-read-stays-canonical", leg_line_read_stays_canonical),
    ]
    failures = []
    for name, leg in legs:
        try:
            print(f"PASS {name}: {leg(exe)}")
        except (AssertionError, TimeoutError) as err:
            failures.append(name)
            detail = str(err).splitlines()[0][:300]
            print(f"FAIL {name}: {detail}")
    if failures:
        print(f"FAILED: {', '.join(failures)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
