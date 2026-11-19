#!/usr/bin/env python3
"""PTY + process-group orchestration driver for the rubash#3321
background-process-group face (the ctty-carrying harness the issue's
gate note demands: the shape is unreachable in a sealed no-ctty test
environment, so the harness builds it).

GNU anchor (jobs.c:4731-4869 initialize_job_control, bash 5.3.20):
the shell takes shell_tty (dup(stderr), or open("/dev/tty") when
forced_interactive and stderr is not a tty), then loops on
tcgetpgrp: while it is not the foreground process group it restores
SIGTTIN to DFL and kills itself with SIGTTIN (jobs.c:4786-4810) —
i.e. it STOPS until foregrounded — and after 17 unheeded kills it
bails with "initialize_job_control: no job control in background"
plus "no job control in this shell", job_control = 0. A shell that
only asks "is stderr a tty / does /dev/tty open" never stops and
never reports the failure.

Harness shape (one leg = one fresh pty session):
  * driver opens a pty pair;
  * a session-leader child (setsid + TIOCSCTTY) owns the pty as its
    controlling terminal and stays the foreground process group;
  * the shell under test is forked from it into a NEW process group
    (setpgid(0, 0)) — a background group of the same session, stdio
    on the slave — or, for the control leg, into the leader's own
    (foreground) group;
  * the leader waitpids the shell with WUNTRACED and reports stop /
    exit / signal events to the driver over a pipe, and on the
    driver's "fg" command hands the terminal to the shell's group
    (tcsetpgrp) and SIGCONTs it.

Legs (each prints one `leg=PASS` / `leg=FAIL detail` line):
  bg-stop-then-fg   background group, SIGTTIN default: the shell must
                    STOP (SIGTTIN) at startup; after fg+CONT it runs
                    with job control (FLAGS=himBHc), no diagnostics.
  bg-sigttin-blocked
                    background group, SIGTTIN blocked: the shell must
                    NOT stop; it must bail with the two GNU
                    diagnostics and FLAGS=hiBHc (no m).
  bg-stderr-pipe    background group, SIGTTIN default, stderr piped
                    to a file (not a tty): same stop-then-fg shape,
                    diagnostics (if any) read from the pipe.
  fg-control        foreground group: FLAGS=himBHc, no diagnostics,
                    no stop.

Exit code 0 = every leg passed; 1 = a leg failed; 2 = no POSIX pty
available (or no python pty module).

Usage: python3 pty_background_pgrp_probe.py <shell-binary> [leg ...]
The shell is run as `<binary> -i -c 'echo FLAGS=$-'` (legs above)
with --norc/--noprofile omitted on purpose: the oracle and rubash
both run the same argv; HOME is pointed at a temp dir so no user
rcfile can leak in.
"""

import os
import pty
import select
import signal
import subprocess
import sys
import tempfile
import termios
import time

if os.name != "posix":
    print("SKIP not a POSIX host")
    sys.exit(2)

ORACLE_HINT = "GNU bash 5.3.20 oracle: ~/workspace/rubash-work/worktrees/issue-3308/third_party/bash/bash"


class PtySession:
    """One pty session whose leader stays foreground; the shell under
    test runs in a chosen process group of that session."""

    def __init__(self, exe, background=True, block_sigttin=False,
                 stderr_pipe=False):
        self.exe = exe
        self.background = background
        self.block_sigttin = block_sigttin
        self.stderr_pipe = stderr_pipe
        self.master, self.slave = pty.openpty()
        # leader -> driver: one status line per waitpid event
        self.evt_r, self.evt_w = os.pipe()
        # driver -> leader: "fg\n" to foreground+CONT the shell
        self.cmd_r, self.cmd_w = os.pipe()
        self.tmpdir = tempfile.mkdtemp(prefix="rubash-i3321-")
        self.shell_pid = None
        self.shell_pgid = None
        self.leader_pid = None
        self._buf = ""
        self._evt_buf = b""

    def _leader(self):
        """Session leader: acquire the ctty, fork the shell, report."""
        try:
            os.close(self.master)
            os.close(self.evt_r)
            os.close(self.cmd_w)
            os.setsid()
            # Acquire the slave as controlling terminal.
            import fcntl
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
            # stdin/stdout(/stderr) on the slave for the shell.
            err_fd = self.slave
            err_file = None
            if self.stderr_pipe:
                err_file = os.path.join(self.tmpdir, "stderr.txt")
                err_fd = os.open(err_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
            pid = os.fork()
            if pid == 0:
                # --- shell under test ---
                try:
                    os.close(self.evt_w)
                    os.close(self.cmd_r)
                    if self.background:
                        os.setpgid(0, 0)
                    if self.block_sigttin:
                        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTTIN})
                    os.dup2(self.slave, 0)
                    os.dup2(self.slave, 1)
                    os.dup2(err_fd, 2)
                    env = {
                        "PATH": "/usr/bin:/bin",
                        "TERM": "xterm",
                        "HOME": self.tmpdir,
                    }
                    os.execve(self.exe, [self.exe, "-i", "-c", "echo FLAGS=$-"], env)
                except BaseException:
                    os._exit(127)
            # --- leader: wait with WUNTRACED, report, obey fg ---
            self.shell_pid = pid
            self.shell_pgid = pid if self.background else os.getpgid(0)
            os.write(self.evt_w, f"spawned {pid} {self.shell_pgid}\n".encode())
            stopped_reported = False
            while True:
                wpid, status = os.waitpid(pid, os.WUNTRACED)
                if os.WIFSTOPPED(status):
                    sig = os.WSTOPSIG(status)
                    os.write(self.evt_w, f"stopped {sig}\n".encode())
                    stopped_reported = True
                    # Wait for the driver's fg command.
                    cmd = b""
                    while not cmd.endswith(b"\n"):
                        chunk = os.read(self.cmd_r, 1)
                        if not chunk:
                            os._exit(0)
                        cmd += chunk
                    if cmd.strip() == b"fg":
                        os.tcsetpgrp(self.slave, self.shell_pgid)
                        os.killpg(self.shell_pgid, signal.SIGCONT)
                    continue
                if os.WIFEXITED(status):
                    os.write(self.evt_w, f"exited {os.WEXITSTATUS(status)}\n".encode())
                    break
                if os.WIFSIGNALED(status):
                    os.write(self.evt_w, f"signaled {os.WTERMSIG(status)}\n".encode())
                    break
            os._exit(0)
        except BaseException:
            try:
                os._exit(1)
            except BaseException:
                pass

    def start(self):
        self.leader_pid = os.fork()
        if self.leader_pid == 0:
            self._leader()
        os.close(self.slave)
        os.close(self.evt_w)
        os.close(self.cmd_r)
        # Read the spawn event (with a timeout so a dead leader fails fast).
        line = self._read_evt_line(timeout=10.0)
        if line is None or not line.startswith("spawned"):
            raise RuntimeError(f"leader did not spawn shell: {line!r}")
        _, pid, pgid = line.split()
        self.shell_pid, self.shell_pgid = int(pid), int(pgid)

    def _read_evt_line(self, timeout):
        end = time.time() + timeout
        while b"\n" not in self._evt_buf:
            remaining = end - time.time()
            if remaining <= 0:
                return None
            r, _, _ = select.select([self.evt_r], [], [], remaining)
            if self.evt_r in r:
                try:
                    data = os.read(self.evt_r, 4096)
                except OSError:
                    return None
                if not data:
                    return None
                self._evt_buf += data
        line, self._evt_buf = self._evt_buf.split(b"\n", 1)
        return line.decode("utf-8", "backslashreplace")

    def pump(self, timeout):
        """Drain the master for `timeout` seconds; return new text."""
        end = time.time() + timeout
        got = b""
        while time.time() < end:
            r, _, _ = select.select([self.master], [], [], 0.1)
            if self.master in r:
                try:
                    data = os.read(self.master, 4096)
                except OSError:
                    break
                if not data:
                    break
                got += data
        return got.decode("utf-8", "backslashreplace")

    def text(self):
        return self._buf

    def wait_event(self, timeout=10.0):
        """Wait for the next leader event line, pumping the master meanwhile."""
        end = time.time() + timeout
        while time.time() < end:
            line = self._read_evt_line(timeout=0.2)
            if line is not None:
                return line
            self._buf += self.pump(0.2)
        return None

    def fg(self):
        os.write(self.cmd_w, b"fg\n")

    def stderr_text(self):
        path = os.path.join(self.tmpdir, "stderr.txt")
        try:
            with open(path, "rb") as f:
                return f.read().decode("utf-8", "backslashreplace")
        except OSError:
            return ""

    def close(self):
        try:
            if self.leader_pid:
                os.kill(self.leader_pid, signal.SIGKILL)
                os.waitpid(self.leader_pid, 0)
        except Exception:
            pass
        for fd in (self.master, self.evt_r, self.cmd_w):
            try:
                os.close(fd)
            except OSError:
                pass


def leg_bg_stop_then_fg(exe):
    s = PtySession(exe, background=True)
    try:
        s.start()
        # The shell must STOP with SIGTTIN at startup (GNU jobs.c:4786-4810).
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("stopped"), \
            f"expected stop event, got {evt!r} text={s.text()!r}"
        assert evt.strip() == "stopped 21", f"expected SIGTTIN(21) stop, got {evt!r}"
        assert "FLAGS=" not in s.text(), "shell ran before being foregrounded"
        s.fg()
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("exited"), \
            f"expected exit after fg, got {evt!r}"
        s._buf += s.pump(1.0)
        text = s.text()
        assert "FLAGS=himBHc" in text, f"expected himBHc after fg, got {text!r}"
        assert "no job control in this shell" not in text, f"unexpected diagnostics: {text!r}"
        assert "cannot set terminal process group" not in text, f"unexpected diagnostics: {text!r}"
    finally:
        s.close()


def leg_bg_sigttin_blocked(exe):
    s = PtySession(exe, background=True, block_sigttin=True)
    try:
        s.start()
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("exited"), \
            f"blocked SIGTTIN must not stop the shell, got {evt!r}"
        s._buf += s.pump(1.0)
        text = s.text()
        assert "FLAGS=hiBHc" in text, f"expected hiBHc (m dropped), got {text!r}"
        assert "initialize_job_control: no job control in background" in text, \
            f"missing background diagnostic: {text!r}"
        # jobs.c:4797 bails with `goto just_bail`, which SKIPS the
        # internal_error "no job control in this shell" line — that
        # second diagnostic must NOT appear on this face.
        assert "no job control in this shell" not in text, \
            f"background bail must not print the second diagnostic: {text!r}"
    finally:
        s.close()


def leg_bg_stderr_pipe(exe):
    s = PtySession(exe, background=True, stderr_pipe=True)
    try:
        s.start()
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("stopped"), \
            f"expected stop event, got {evt!r} text={s.text()!r}"
        s.fg()
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("exited"), \
            f"expected exit after fg, got {evt!r}"
        s._buf += s.pump(1.0)
        text = s.text()
        err = s.stderr_text()
        assert "FLAGS=himBHc" in text, f"expected himBHc after fg, got {text!r}"
        assert "no job control in this shell" not in err, f"unexpected diagnostics: {err!r}"
    finally:
        s.close()


def leg_fg_control(exe):
    s = PtySession(exe, background=False)
    try:
        s.start()
        evt = s.wait_event(timeout=10.0)
        assert evt is not None and evt.startswith("exited"), \
            f"foreground shell must not stop, got {evt!r}"
        s._buf += s.pump(1.0)
        text = s.text()
        assert "FLAGS=himBHc" in text, f"expected himBHc, got {text!r}"
        assert "no job control in this shell" not in text, f"unexpected diagnostics: {text!r}"
    finally:
        s.close()


LEGS = {
    "bg-stop-then-fg": leg_bg_stop_then_fg,
    "bg-sigttin-blocked": leg_bg_sigttin_blocked,
    "bg-stderr-pipe": leg_bg_stderr_pipe,
    "fg-control": leg_fg_control,
}


def main():
    if len(sys.argv) < 2:
        print("usage: pty_background_pgrp_probe.py <shell-binary> [leg ...]")
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
        try:
            LEGS[leg](exe)
            print(f"{leg}=PASS", flush=True)
        except Exception as err:
            failed.append(leg)
            detail = repr(err).splitlines()[0][:300]
            print(f"{leg}=FAIL {detail}", flush=True)
        time.sleep(0.1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
