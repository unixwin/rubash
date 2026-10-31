#!/usr/bin/env python3
"""POSIX-pty execution-level driver for the rubash#2233 hangup
regressions: a shell whose controlling terminal hangs up mid-session
(pty master closed) must die BY SIGHUP where GNU does — not fold the
terminal's EIO into a graceful EOF exit, and not orphan its jobs.

Wiring: session leader, stdin = pty slave, stdout/stderr = pipes.
The wiring selects the route exactly as GNU's shell.c:541-547 does:
no `-i` and stderr not a tty -> the shell is a NON-interactive
SCRIPT reader on the tty; `-i` forces the interactive reader. Both
routes are pinned, each against its own GNU control (bash 5.3.20,
same wiring):

Script route (args: --norc --noprofile):
  GNU acquires the ctty via check_dev_tty (general.c:651, from main
  shell.c:410) even for script shells, so the kernel delivers SIGHUP
  on hangup and the untrapped shell dies by it. An EXIT trap arms
  the termsig set: the trap runs and the shell still dies by the
  signal. A trapped or ignored SIGHUP cannot preempt the dead
  terminal: the parser's READERR arm (parse.y shell_getc,
  FATAL_READERROR) reports `script file read error` and exits 128,
  the queued HUP trap unrun.

Interactive route (args: --norc --noprofile -i):
  The interactive termsig installation (sig.c:246/291, :315-316)
  catches SIGHUP; termsig_handler (sig.c:590) saves history, hangs
  up every job (jobs.c:1783 hangup_all_jobs), runs the EXIT trap,
  and dies by the signal. A trapped or ignored SIGHUP leaves the
  dead terminal to the reader, which folds it like EOF: graceful
  exit with the last status (the trap does not run on this path).

Login legs add `shopt -s huponexit` semantics (shell.c:1017-1020
exit_shell): an interactive LOGIN shell with the shopt hangs its
jobs up on a normal exit; without it they survive. Both legs run
`set -m` first so the job has its own process group in both shells
— that isolates the shopt variable from kernel leader-exit
delivery, which targets the foreground group only.

Baseline (pre-fix) rubash: no controlling terminal is ever acquired
(tty_nr=0), so no SIGHUP is delivered on master close in any cell:
the script reader folds EIO into the last status (0/1), the
interactive reader prints "exit" and leaves 0, jobs survive, and
huponexit has no behavior.

Exit code 0 = every leg passed; 1 = a leg failed; 2 = no POSIX pty
available (Windows hosts skip; there is no SIGHUP there).

Usage: python3 pty_hangup_probe.py <rubash-binary> [leg ...]
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

SCRIPT_ARGS = ("--norc", "--noprofile")
INTERACTIVE_ARGS = ("--norc", "--noprofile", "-i")
# GNU rejects long options after an option cluster carrying `i`
# (usage error, verified against bash 5.3.20); `--login -i` is the
# accepted spelling. HOME is a fresh tempdir, so no rc/profile
# content can leak into the legs from either shell.
LOGIN_ARGS = ("--login", "-i")


def pid_alive(pid):
    """True while `pid` is a live process (a zombie does not count:
    its /proc cmdline is empty where /proc exists)."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as fh:
            return bool(fh.read())
    except OSError:
        return True


def wait_pid_dead(pid, timeout=6.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not pid_alive(pid):
            return True
        time.sleep(0.1)
    return not pid_alive(pid)


class Session:
    """One engine session: stdin on a canonical pty slave (its own
    session, so the slave can become the controlling terminal),
    stdout+stderr on a pipe the driver drains."""

    def __init__(self, exe, args):
        self.master, slave = pty.openpty()
        self.workdir = tempfile.mkdtemp(prefix="hangup2233-")
        self.proc = subprocess.Popen(
            [exe, *args],
            stdin=slave,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            close_fds=True,
            cwd=self.workdir,
            env={"PATH": "/usr/bin:/bin", "TERM": "xterm",
                 "HOME": self.workdir},
        )
        os.close(slave)
        self.out_fd = self.proc.stdout.fileno()
        self.chunks = []

    def _pump(self, timeout):
        end = time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.out_fd], [], [], 0.1)
            if self.out_fd in r:
                try:
                    data = os.read(self.out_fd, 4096)
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

    def marker(self, needle, timeout=15.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._pump(0.2)
            if needle in self.text():
                return True
        return False

    def ready(self):
        self.write("echo BOOT2233\n")
        assert self.marker("BOOT2233"), \
            f"shell never ran a command; tail={self.text()[-200:]!r}"

    def wait_exit(self, timeout=12.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._pump(0.2)
            rc = self.proc.poll()
            if rc is not None:
                return rc
        return None

    def close_master(self):
        os.close(self.master)
        self.master = -1

    def close(self):
        try:
            if self.proc.poll() is None:
                self.proc.kill()
            self.proc.wait()
        except Exception:
            pass
        if self.master >= 0:
            try:
                os.close(self.master)
            except OSError:
                pass


def spawn_job(session):
    """Start a background sleep and learn its pid from the shell.
    The echoed input line also carries the marker (with `$!`
    unexpanded), so only a line whose value is all digits counts."""
    session.write("sleep 60 & echo JOBPID2233=$!\n")
    deadline = time.time() + 15.0
    while time.time() < deadline:
        session._pump(0.2)
        for line in session.text().splitlines():
            if "JOBPID2233=" in line:
                value = line.rsplit("=", 1)[1].strip()
                if value.isdigit():
                    return int(value)
    raise AssertionError("job pid not found in output")


def arm_file_trap(session, sig, fname):
    path = os.path.join(session.workdir, fname)
    session.write(f"trap 'echo fired >> {path}' {sig}\n")
    session.write("echo TRAPSET2233\n")
    assert session.marker("TRAPSET2233"), f"{sig} trap arm marker missing"
    return path


def file_fired(path, timeout=5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if os.path.exists(path):
            with open(path) as fh:
                if "fired" in fh.read():
                    return True
        time.sleep(0.1)
    return False


# --- shared leg shapes -------------------------------------------------

def shape_close_dies_by_sighup(s):
    s.close_master()
    rc = s.wait_exit()
    assert rc == -1, f"exit status={rc!r} (want -1, death by SIGHUP)"


def shape_after_read_eof_close(s):
    s.write("read x; echo R2233 RC=$?\n")
    time.sleep(0.5)
    s.write("\x04")
    assert s.marker("R2233 RC=1"), f"read EOF leg broke: {s.text()[-200:]!r}"
    s.close_master()
    rc = s.wait_exit()
    assert rc == -1, f"exit status={rc!r} (want -1, death by SIGHUP; " \
        "the baseline folded the hangup into a graceful exit)"


def shape_direct_kill(s):
    s.write("kill -HUP $$\n")
    rc = s.wait_exit()
    assert rc == -1, f"exit status={rc!r} (want -1, death by SIGHUP)"


def shape_kills_job(s):
    pid = spawn_job(s)
    assert pid_alive(pid), "job not alive before the hangup"
    s.close_master()
    rc = s.wait_exit()
    assert rc == -1, f"shell exit status={rc!r} (want -1)"
    assert wait_pid_dead(pid), f"job {pid} survived the hangup"


def shape_exit_trap(s):
    path = arm_file_trap(s, "EXIT", "exit-trap-fired")
    s.close_master()
    rc = s.wait_exit()
    assert rc == -1, f"exit status={rc!r} (want -1, death by SIGHUP)"
    assert file_fired(path), "EXIT trap did not run on the hangup path"


def shape_trapped_direct(s):
    # Control, both routes: with the terminal alive, a trapped
    # SIGHUP runs its trap and the shell survives (the hangup work
    # must not break ordinary trap semantics).
    path = arm_file_trap(s, "HUP", "hup-trap-fired")
    s.write("kill -HUP $$\n")
    assert file_fired(path), "HUP trap did not run on a direct delivery"
    assert s.proc.poll() is None, "shell died despite the HUP trap"
    s.write("echo STILLALIVE2233\n")
    assert s.marker("STILLALIVE2233"), "shell not responsive after trap"


# --- script-route legs ---------------------------------------------------

def leg_s_trapped(s):
    # GNU control: 128 + the read-error diagnostic; whether the
    # queued trap runs is not pinned (GNU's did not), the status and
    # diagnostic are the stable verdicts.
    arm_file_trap(s, "HUP", "hup-trap-fired")
    s.close_master()
    rc = s.wait_exit()
    assert rc == 128, f"exit status={rc!r} (want 128 read-error exit)"
    assert "script file read error" in s.text(), \
        f"read-error diagnostic missing: {s.text()[-200:]!r}"


def leg_s_ignored(s):
    s.write("trap '' HUP\n")
    s.write("echo TRAPSET2233\n")
    assert s.marker("TRAPSET2233"), "ignore arm marker missing"
    s.close_master()
    rc = s.wait_exit()
    assert rc == 128, f"exit status={rc!r} (want 128 read-error exit)"
    assert "script file read error" in s.text(), \
        f"read-error diagnostic missing: {s.text()[-200:]!r}"


# --- interactive-route legs ----------------------------------------------

def shape_i_trapped_like(s, arm):
    # GNU control: the reader folds the dead terminal like EOF —
    # graceful exit with the last status (0 here). Not a signal
    # death, not the script route's 128.
    if arm == "trap":
        arm_file_trap(s, "HUP", "hup-trap-fired")
    else:
        s.write("trap '' HUP\n")
        s.write("echo TRAPSET2233\n")
        assert s.marker("TRAPSET2233"), "ignore arm marker missing"
    s.close_master()
    rc = s.wait_exit()
    assert rc == 0, f"exit status={rc!r} (want 0, graceful EOF fold)"


def leg_i_trapped(s):
    shape_i_trapped_like(s, "trap")


def leg_i_ignored(s):
    shape_i_trapped_like(s, "ignore")


def leg_i_trapped_direct(s):
    shape_trapped_direct(s)
    s.write("\x04")
    rc = s.wait_exit()
    assert rc == 0, f"prompt EOF exit status={rc!r}"


# --- login legs ------------------------------------------------------------

def leg_huponexit_login(s):
    s.write("set -m\n")
    s.write("shopt -s huponexit\n")
    pid = spawn_job(s)
    s.write("exit\n")
    rc = s.wait_exit()
    assert rc == 0, f"login exit status={rc!r}"
    assert wait_pid_dead(pid), \
        f"job {pid} survived a huponexit login shell's exit"


def leg_huponexit_off(s):
    s.write("set -m\n")
    pid = spawn_job(s)
    s.write("exit\n")
    rc = s.wait_exit()
    assert rc == 0, f"login exit status={rc!r}"
    time.sleep(0.5)
    alive = pid_alive(pid)
    if alive:
        os.kill(pid, 9)
    assert alive, f"job {pid} died without huponexit (control leg)"


LEGS = {
    # script route (no -i; stderr not a tty)
    "s-hangup-at-prompt": (shape_close_dies_by_sighup, SCRIPT_ARGS),
    "s-hangup-after-read-eof": (shape_after_read_eof_close, SCRIPT_ARGS),
    "s-hangup-direct-kill": (shape_direct_kill, SCRIPT_ARGS),
    "s-hangup-kills-job": (shape_kills_job, SCRIPT_ARGS),
    "s-hangup-exit-trap": (shape_exit_trap, SCRIPT_ARGS),
    "s-hangup-trapped": (leg_s_trapped, SCRIPT_ARGS),
    "s-hangup-ignored": (leg_s_ignored, SCRIPT_ARGS),
    "s-hangup-trapped-direct": (shape_trapped_direct, SCRIPT_ARGS),
    # interactive route (-i forced)
    "i-hangup-at-prompt": (shape_close_dies_by_sighup, INTERACTIVE_ARGS),
    "i-hangup-after-read-eof": (shape_after_read_eof_close, INTERACTIVE_ARGS),
    "i-hangup-direct-kill": (shape_direct_kill, INTERACTIVE_ARGS),
    "i-hangup-kills-job": (shape_kills_job, INTERACTIVE_ARGS),
    "i-hangup-runs-exit-trap": (shape_exit_trap, INTERACTIVE_ARGS),
    "i-hangup-trapped": (leg_i_trapped, INTERACTIVE_ARGS),
    "i-hangup-ignored": (leg_i_ignored, INTERACTIVE_ARGS),
    "i-hangup-trapped-direct": (leg_i_trapped_direct, INTERACTIVE_ARGS),
    # login / huponexit
    "huponexit-login": (leg_huponexit_login, LOGIN_ARGS),
    "huponexit-off": (leg_huponexit_off, LOGIN_ARGS),
}


def main():
    if len(sys.argv) < 2:
        print("usage: pty_hangup_probe.py <rubash-binary> [leg ...]")
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
        fn, args = LEGS[leg]
        session = Session(exe, args)
        try:
            session.ready()
            fn(session)
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
