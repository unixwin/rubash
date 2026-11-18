#!/usr/bin/env python3
"""POSIX-pty driver for the rubash#3330 `help` width regression: run a
shell's bare `help` listing under a pseudo-terminal whose winsize is
set BEFORE the child execs, with COLUMNS/LINES unset, and print the
raw listing (CRLF normalized to LF) on stdout.

GNU anchor (bash 5.3.20): general.c:1461 default_columns takes COLUMNS
when it parses positive, else the window size from
get_new_window_size (lib/sh/winsize.c: tcgetwinsize on input_tty(),
i.e. the controlling terminal else stderr), else 80. A shell whose
unix winsize query is a stub prints the 80-column grid at every
window width; a fixed one prints the grid for the pty's width.

Usage: python3 pty_help_width_probe.py <shell-binary> <cols> [rows]
Exit code 0 = listing printed; 1 = driver/shell error; 2 = no POSIX
pty available (the caller skips the leg).
"""

import fcntl
import os
import struct
import sys
import termios

try:
    import pty
except ImportError:
    print("SKIP pty module missing")
    sys.exit(2)

if os.name != "posix":
    print("SKIP not a POSIX host")
    sys.exit(2)


def main():
    if len(sys.argv) < 3:
        print(
            "usage: pty_help_width_probe.py <shell-binary> <cols> [rows]",
            file=sys.stderr,
        )
        return 2
    shell = sys.argv[1]
    try:
        cols = int(sys.argv[2])
        rows = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    except ValueError:
        print("cols/rows must be integers", file=sys.stderr)
        return 2
    if cols <= 0 or rows <= 0:
        print("cols/rows must be positive", file=sys.stderr)
        return 2

    master, slave = pty.openpty()
    # Set the winsize on the slave BEFORE forking: the child execs
    # straight into the query, so a post-fork set would race it.
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    pid = os.fork()
    if pid == 0:
        try:
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0)
            os.dup2(slave, 1)
            os.dup2(slave, 2)
            os.close(master)
            os.close(slave)
            env = dict(os.environ)
            env.pop("COLUMNS", None)
            env.pop("LINES", None)
            # Pin the single-byte locale: the nail's grids are the
            # dispcolumn (LC_ALL=C) render. Under a UTF-8 leg locale
            # the shell selects the wdispcolumn path instead, which
            # keeps one more character at the truncation boundary
            # (help.rs listing_cell; GNU help.def wdispcolumn vs
            # dispcolumn) — same product, different grid. Without the
            # pin the 100-column row for `select` loses its `>`.
            env["LC_ALL"] = "C"
            os.execve(shell, [shell, "-c", "help"], env)
        except BaseException:
            os._exit(127)
    os.close(slave)
    out = b""
    try:
        while True:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
    finally:
        os.close(master)
    _, status = os.waitpid(pid, 0)
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        print(f"shell exited with status {status}", file=sys.stderr)
        return 1
    sys.stdout.write(out.replace(b"\r\n", b"\n").decode("utf-8", "replace"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
