#!/usr/bin/env python3
"""Marker-leak golden, PTY sibling driver (rubash#2885 E6).

Drives an interactive rubash through a PTY for the channels that only
exist interactively — prompt rendering (PS1/PS2/PS4), history-file
storage, and `read -p` — mirroring the #2879 sweep's PTY battery
(`pty_prompts.py`). Harness shape: /usr/bin/bash -i as session leader /
job-control parent under the PTY (pty.openpty + fork + setsid +
TIOCSCTTY, never pty.fork); the shell under test runs as the parent's
foreground job, so job-control state is the real terminal shape.

Usage: marker_leak_golden_pty.py <rubash-binary> <leg> [<leg> ...]
Prints one `LEG <name> PASS|FAIL <reasons>` line per leg; exit 0 iff
every requested leg passes, 2 when the host cannot run the harness
(no /usr/bin/bash) — the Rust wrapper treats 2 as a skip.
"""
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time

PARENT_BASH = "/usr/bin/bash"
E314_UTF8 = "\uE314".encode("utf-8")        # EE 90 B4
E400_E314 = "\uE400\uE314".encode("utf-8")  # EE 90 80 EE 90 B4
E401_UTF8 = "\uE401".encode("utf-8")        # EE 90 81
WORKDIR = tempfile.mkdtemp(prefix="rubash-marker-pty-")


def drain(fd):
    out = b""
    while True:
        r, _, _ = select.select([fd], [], [], 0.15)
        if not r:
            break
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
    return out


def drain_until_quiet(fd, cap=6.0, quiet=0.35):
    """Read until the pty has been silent for `quiet` seconds (or `cap`
    elapses). Fixed sleeps race under build load; output quiescence is
    the condition that actually means 'the shell finished this step'."""
    out = b""
    deadline = time.time() + cap
    last_growth = time.time()
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            last_growth = time.time()
        elif time.time() - last_growth >= quiet and out:
            break
    return out


def write(fd, s):
    os.write(fd, s.encode() if isinstance(s, str) else s)


def spawn_parent(child_cmd, home):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    pid = os.fork()
    if pid == 0:
        try:
            os.close(master)
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0)
            os.dup2(slave, 1)
            os.dup2(slave, 2)
            if slave > 2:
                os.close(slave)
            os.environ["PS1"] = "PARENT$ "
            os.environ["TERM"] = "xterm"
            os.environ["HOME"] = home
            os.environ["AUDITDIR"] = WORKDIR
            os.execv(PARENT_BASH, ["bash", "--norc", "-i"])
        except Exception as e:  # noqa: BLE001 - report and die
            os.write(2, ("child setup failed: %r\n" % e).encode())
            os._exit(127)
    os.close(slave)
    # Wait for the parent's prompt, then start the child shell and wait
    # for its prompt — condition-based, not fixed sleeps.
    buf = drain_until_quiet(master, cap=10.0)
    write(master, child_cmd + "\n")
    buf += drain_until_quiet(master, cap=10.0)
    return pid, master, buf


def run_session(shell, tag, steps):
    home = os.path.join(WORKDIR, "home-" + tag)
    os.makedirs(home, exist_ok=True)
    pid, master, buf = spawn_parent("PS1='CHILD$ ' %s --norc -i" % shell, home)
    try:
        for send, _settle in steps:
            if send:
                write(master, send)
            buf += drain_until_quiet(master)
        return buf
    finally:
        try:
            write(master, b"exit\n")
            time.sleep(0.3)
        except OSError:
            pass
        try:
            os.kill(pid, signal.SIGKILL)
        except (OSError, ProcessLookupError):
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
        try:
            os.close(master)
        except OSError:
            pass


def hist_file_bytes():
    try:
        with open(os.path.join(WORKDIR, "histfile"), "rb") as fh:
            return fh.read()
    except OSError:
        return b""


CASES = {
    # Sweep s1: PS1 from a command substitution carrying a raw ESC.
    "ps1_comsub_esc": (
        [
            ("PS1='X$(printf \"P\\033Q\")Y '\n", 0.7),
            ("true\n", 0.5),
        ],
        [
            ("prompt carries real ESC byte (P<ESC>Q)", lambda t: b"P\x1bQ" in t),
            ("no E400 pair in transcript", lambda t: E400_E314 not in t),
        ],
    ),
    # Sweep s2: PS1 from a stored variable carrying a raw ESC.
    "ps1_stored_esc": (
        [
            ("v=$(printf 'A\\033B')\n", 0.6),
            ("PS1='<$v> '\n", 0.7),
            ("true\n", 0.5),
        ],
        [
            ("prompt carries stored ESC (A<ESC>B)", lambda t: b"A\x1bB" in t),
            ("no E400 pair in transcript", lambda t: E400_E314 not in t),
        ],
    ),
    # Sweep s3: PS2 from a command substitution carrying a raw ESC.
    "ps2_comsub_esc": (
        [
            ("PS2='C$(printf \"P\\033Q\")D '\n", 0.6),
            ("echo \\\n", 0.7),
            ("hi\n", 0.5),
        ],
        [
            ("PS2 carries real ESC byte (P<ESC>Q)", lambda t: b"P\x1bQ" in t),
            ("no E400 pair in transcript", lambda t: E400_E314 not in t),
        ],
    ),
    # Sweep s5: PS1 from a typed PUA value renders the raw char.
    "ps1_typed_pua": (
        [
            ("v=$(printf '\\uE314')\n", 0.6),
            ("PS1=\"<$v> \"\n", 0.7),
            ("true\n", 0.5),
        ],
        [
            ("prompt carries raw U+E314 char", lambda t: E314_UTF8 in t),
            ("no E400+E314 escape pair in prompt", lambda t: E400_E314 not in t),
        ],
    ),
    # Sweep Q1 (captain-accepted at #2878): a TAB carried by a value
    # renders as a real TAB in PS1; the ANSI_C_IFS_GUARD must not
    # survive into the rendered prompt.
    "ps1_tab_value": (
        [
            ("v=$'A\\tB'\n", 0.6),
            ("PS1='<$v> '\n", 0.7),
            ("true\n", 0.5),
        ],
        [
            ("PS1 renders the TAB as a real TAB byte", lambda t: b"A\tB" in t),
            ("no U+E401 guard in transcript", lambda t: E401_UTF8 not in t),
        ],
    ),
    # Sweep Q1, PS4 leg: xtrace prefix from a TAB-carrying value.
    "ps4_tab_value": (
        [
            ("v=$'A\\tB'\n", 0.6),
            ("PS4='<*$v*> '\n", 0.6),
            ("set -x; true; set +x\n", 0.7),
        ],
        [
            ("PS4 renders the TAB as a real TAB byte", lambda t: b"<*A\tB*>" in t),
            ("no U+E401 guard in transcript", lambda t: E401_UTF8 not in t),
        ],
    ),
    # History: a typed $'\uE314' line is stored as the typed spelling
    # (transport only); no PUA bytes may reach the history file
    # (sweep §2.1 history cell, byte-identical to GNU).
    "hist_typed_line": (
        [
            ("echo $'\\uE314'\n", 0.6),
            ("history -w \"$AUDITDIR/histfile\"\n", 0.7),
        ],
        [
            (
                "history file keeps the typed $'\\uE314' spelling",
                lambda t: b"echo $'\\uE314'" in hist_file_bytes(),
            ),
            (
                "no PUA bytes in the history file",
                lambda t: b"\xee" not in hist_file_bytes(),
            ),
        ],
    ),
    # Sweep s4: `read -p` with a raw ESC in the prompt. Separate leg —
    # the Rust side runs it as an #[ignore]d test while rubash#2883
    # (sweep F4: expansion results are not prompt-decoded) is open.
    "read_p_esc": (
        [
            ("read -p $'R\\033S: ' x\n", 0.7),
            ("hello\n", 0.5),
            ("echo \"got=$x\"\n", 0.5),
        ],
        [
            ("read -p prompt carries real ESC (R<ESC>S)", lambda t: b"R\x1bS" in t),
            ("answer read back", lambda t: b"got=hello" in t),
        ],
    ),
}


def main():
    if not os.path.exists(PARENT_BASH):
        print("SKIP: no /usr/bin/bash job-control parent available")
        return 2
    shell = sys.argv[1]
    legs = sys.argv[2:]
    rc = 0
    for name in legs:
        steps, checks = CASES[name]
        transcript = run_session(shell, name, steps)
        if name == "hist_typed_line":
            # The history write lands asynchronously at the session's
            # pace; poll for the file instead of assuming the settles
            # were enough under load.
            for _ in range(25):
                if hist_file_bytes():
                    break
                time.sleep(0.2)
        fails = [desc for desc, pred in checks if not pred(transcript)]
        if fails:
            rc = 1
            print("LEG %s FAIL %s" % (name, "; ".join(fails)))
            tail = transcript.replace(b"\r", b"")[-300:]
            print("  transcript tail: %r" % tail)
        else:
            print("LEG %s PASS" % name)
    return rc


if __name__ == "__main__":
    sys.exit(main())
