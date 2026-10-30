#!/usr/bin/env python3
"""POSIX-pty execution-level driver for the niubash#684 Ctrl-D read
regressions (the interop face: an interactive engine whose stdin is a
canonical pty, driven byte by byte).

GNU anchor (WSL GNU Bash 5.3.20, same script): at a canonical tty the
kernel line discipline turns the VEOF char (Ctrl-D) into a zero-length
read — `read x` at an empty line hits EOF (builtins/read.def:767
eof=1, :949 EXECUTION_FAILURE) and the SHELL SURVIVES for the next
command; mid-line, VEOF only flushes the buffered prefix (`abc` ^D
`def` reads "abcdef", rc 0); at the prompt, VEOF exits the shell with
the last status. Pre-fix, the engine's first stdin transport DID see
the EOF but the acquisition cascade asked a second blocking reader on
the same descriptor, which swallowed the EOF and consumed the NEXT
typed line as the read's record.

Legs:
  ctrld-read-eof    `read x` + ^D -> X=[] rc=1, then a fresh command
                    still executes (EOF ends only the current input).
  ctrld-read-flush  control: `read y` + `abc` ^D `def` -> Y=[abcdef]
                    rc=0 (VEOF flush is not EOF mid-record).
  ctrld-prompt-eof  control: ^D at an empty prompt exits the shell
                    with status 0.

Exit code 0 = every leg passed; 1 = a leg failed; 2 = no POSIX pty
available (Windows — the ConPTY sibling driver covers that host).

Usage: python3 pty_read_probe.py <rubash-binary> [leg ...]
Prints one `leg=PASS` / `leg=FAIL detail` line per requested leg.
"""

import os
import select
import subprocess
import sys
import tempfile
import time

try:
    import pty
except ImportError:
    print("SKIP pty module missing")
    sys.exit(2)

if os.name != "posix":
    print("SKIP not a POSIX host")
    sys.exit(2)


class Session:
    """One interactive engine session on a canonical pty."""

    def __init__(self, exe):
        self.master, slave = pty.openpty()
        self.proc = subprocess.Popen(
            [exe, "--norc", "--noprofile"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
            close_fds=True,
            cwd=tempfile.gettempdir(),
            env={"PATH": "/usr/bin:/bin", "TERM": "xterm",
                 "HOME": tempfile.gettempdir()},
        )
        os.close(slave)
        self.chunks = []

    def _pump(self, timeout):
        end = time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.master], [], [], 0.1)
            if self.master in r:
                try:
                    data = os.read(self.master, 4096)
                except OSError:
                    return False
                if not data:
                    return False
                self.chunks.append(data)
        return True

    def text(self):
        return b"".join(self.chunks).decode("utf-8", "backslashreplace")

    def write(self, keys):
        os.write(self.master, keys.encode())

    def ready(self, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._pump(0.2)
            if "5.3#" in self.text():
                return
        raise TimeoutError(f"no prompt; tail={self.text()[-200:]!r}")

    def marker(self, needle, timeout=12.0):
        """Wait for an output line carrying `needle` that is not the
        echoed command line (the echo always contains 'echo')."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._pump(0.2)
            for line in self.text().split("\n"):
                if needle in line and "echo" not in line:
                    return line.strip()
        return None

    def wait_exit(self, timeout=12.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._pump(0.2)
            rc = self.proc.poll()
            if rc is not None:
                return rc
        return None

    def close(self):
        try:
            if self.proc.poll() is None:
                self.proc.kill()
            self.proc.wait()
        except Exception:
            pass
        try:
            os.close(self.master)
        except OSError:
            pass


def leg_ctrld_read_eof(s):
    s.write('read x; echo "D0#""RC=$? X=[$x]"\n')
    time.sleep(0.5)
    s.write("\x04")
    line = s.marker("D0#RC=")
    assert line and "RC=1" in line and "X=[]" in line, f"marker={line!r}"
    # The EOF ended only the read: the shell takes the next command.
    s.write("echo ZZALIVE684\n")
    line = s.marker("ZZALIVE684")
    assert line, "shell did not survive the read EOF"
    # And a prompt-level Ctrl-D still exits the shell, status 0.
    s.write("\x04")
    rc = s.wait_exit()
    assert rc == 0, f"prompt EOF exit status={rc!r}"


def leg_ctrld_read_flush(s):
    s.write('read y; echo "DF#""RC=$? Y=[$y]"\n')
    time.sleep(0.5)
    s.write("abc")
    s.write("\x04")
    s.write("def\n")
    line = s.marker("DF#RC=")
    assert line and "RC=0" in line and "Y=[abcdef]" in line, \
        f"marker={line!r}"


def leg_ctrld_prompt_eof(s):
    s.write("\x04")
    rc = s.wait_exit()
    assert rc == 0, f"prompt EOF exit status={rc!r}"


LEGS = {
    "ctrld-read-eof": leg_ctrld_read_eof,
    "ctrld-read-flush": leg_ctrld_read_flush,
    "ctrld-prompt-eof": leg_ctrld_prompt_eof,
}


def main():
    if len(sys.argv) < 2:
        print("usage: pty_read_probe.py <rubash-binary> [leg ...]")
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
