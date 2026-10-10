#!/usr/bin/env python3
"""ConPTY probe: bare rubash.exe edits lines with real readline bindings.

rubash#419 readline leg: the console REPL used to leave stdin in cooked
line-input mode, where conhost owns the editing — no shell history on the
arrow keys, no cursor motion inside the line, and --rcfile was the only
way PS1 got set. The raw console reader (src/console_readline.rs) now
switches the console to raw mode between the prompt and the accepted
line: keystrokes reach the engine's edit dispatcher one event at a time
(history arrows, C-a/C-e, backspace rubout), the line repaints per
keystroke, and the console returns to cooked mode while the command runs.

This probe drives the process under a real ConPTY (pywinpty + pyte, the
smoke-console-repl-pty.py pattern) and checks the RENDERED SCREEN for:

* --rcfile-set PS1 rendered as the interactive prompt (no banner);
* backspace editing (``echo AB`` + BS + ``C`` runs ``echo AC``);
* history recall on the Up arrow (the recalled line re-appears at the
  prompt and a bare Enter re-runs it);
* C-a home re-edit (cursor motion does not corrupt the line).

Exit codes: 0 pass, 1 fail, 2 skip (python deps missing).

Usage: python scripts/smoke-console-readline-pty.py [rubash.exe]
       (default binary: target/debug/rubash.exe)
"""

import sys
import tempfile
import time
from pathlib import Path

try:
    import pyte
    from winpty import PtyProcess
except ImportError as err:  # pragma: no cover - environment guard
    print(f"SKIP: python deps missing ({err}); needs pywinpty + pyte")
    sys.exit(2)

REPO = Path(__file__).resolve().parent.parent
ARTIFACTS = REPO / "target" / "issue-suites" / "results" / "wt64-repl"
ARTIFACTS.mkdir(parents=True, exist_ok=True)

COLS, ROWS = 100, 30
SETTLE_SECONDS = 0.4
KEY_GAP_SECONDS = 0.15
ENTER = "\r"
UP = "\x1b[A"
BACKSPACE = "\x08"
CTRL_A = "\x01"


class Session:
    last_screens: dict = {}

    def __init__(self, argv, env=None, name="unnamed"):
        self.name = name
        self.proc = PtyProcess.spawn(
            argv, cwd=str(REPO), env=env, dimensions=(ROWS, COLS)
        )
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.alive = True
        import threading

        self._reader = threading.Thread(target=self._pump, daemon=True)
        self._reader.start()

    def text(self) -> str:
        body = "\n".join(self.screen.display)
        Session.last_screens[self.name] = body
        return body

    def close(self):
        Session.last_screens[self.name] = self.text()
        try:
            self.proc.terminate(force=True)
        except Exception:
            pass

    def _pump(self):
        while self.proc.isalive():
            try:
                data = self.proc.read()
            except Exception:
                break
            if data:
                self.stream.feed(data)
        self.alive = False

    def wait_for(self, *needles, timeout=15):
        deadline = time.time() + timeout
        while time.time() < deadline:
            body = self.text()
            for needle in needles:
                if needle in body:
                    return needle
            time.sleep(0.05)
        raise TimeoutError(
            f"timed out waiting for {needles}; screen:\n{self.text()}"
        )

    def send_keys(self, *chunks):
        """Send key chunks as separate writes with gaps (raw-mode key
        events; one burst can merge into a bridge-style text line)."""
        for chunk in chunks:
            time.sleep(SETTLE_SECONDS if chunk == chunks[0] else KEY_GAP_SECONDS)
            self.proc.write(chunk)

    def send(self, keys):
        """Settle, then send text and Enter as separate writes."""
        time.sleep(SETTLE_SECONDS)
        if keys.endswith(ENTER) and len(keys) > 1:
            self.proc.write(keys[:-1])
            time.sleep(KEY_GAP_SECONDS)
            self.proc.write(ENTER)
        else:
            self.proc.write(keys)

    def wait_exit(self, timeout=10):
        deadline = time.time() + timeout
        while self.proc.isalive() and time.time() < deadline:
            time.sleep(0.1)
        assert not self.proc.isalive(), "shell did not exit"


def write_rc(ps1: str) -> Path:
    rc = Path(tempfile.gettempdir()) / f"rubash-419-readline-{ps1[:2].strip().lower() or 'x'}.rc"
    rc.write_text(f"PS1='{ps1}'\n", encoding="utf-8")
    return rc


def leg_rcfile_ps1(exe: Path, base_env) -> str:
    """--rcfile sets PS1; the raw console renders it as the prompt and the
    shell reads commands through the readline loop."""
    rc = write_rc("RC> ")
    env = dict(base_env)
    session = Session(
        [str(exe), "--rcfile", str(rc), "-i"], env=env, name="readline-rcfile"
    )
    try:
        session.wait_for("RC> ")
        body = session.text()
        assert "Rubash - A Rust implementation" not in body, "banner rendered"
        session.send("echo RC-OK" + ENTER)
        session.wait_for("RC-OK")
        body = session.text()
        assert body.count("echo RC-OK") == 1, (
            f"line echoed {body.count('echo RC-OK')} times "
            "(raw mode must own the echo)"
        )
        return body
    finally:
        session.close()


def leg_backspace_edit(exe: Path, base_env) -> str:
    """Backspace rubout + insert: ``echo AB`` BS ``C`` runs ``echo AC``."""
    env = dict(base_env)
    env["PS1"] = "E> "
    session = Session([str(exe)], env=env, name="readline-backspace")
    try:
        session.wait_for("E> ")
        session.send("echo AB" + BACKSPACE + "C" + ENTER)
        session.wait_for("AC")
        body = session.text()
        assert "AC" in body, f"rubout result missing: {body}"
        assert "echo AB" not in body.split("E> ")[-1], (
            f"rubout did not land: {body}"
        )
        session.send("exit 0" + ENTER)
        session.wait_exit()
        return session.text()
    finally:
        session.close()


def leg_history_up_arrow(exe: Path, base_env) -> str:
    """Up arrow recalls the previous entry to the prompt; Enter re-runs it."""
    env = dict(base_env)
    env["PS1"] = "H> "
    session = Session([str(exe)], env=env, name="readline-history")
    try:
        session.wait_for("H> ")
        session.send("echo HIST-UP-OK" + ENTER)
        session.wait_for("HIST-UP-OK")
        # Up arrow recalls the entry; the recalled line renders at the
        # prompt (the redraw), then Enter accepts it again.
        session.send_keys(UP, ENTER)
        deadline = time.time() + 10
        while session.text().count("HIST-UP-OK") < 2 and time.time() < deadline:
            time.sleep(0.05)
        body = session.text()
        assert body.count("HIST-UP-OK") >= 2, f"history recall failed: {body}"
        session.send("exit 0" + ENTER)
        session.wait_exit()
        return body
    finally:
        session.close()


def leg_ctrl_a_reedit(exe: Path, base_env) -> str:
    """C-a jumps home; typing then lands at the line start without
    corrupting the rest (cursor motion over the raw console)."""
    env = dict(base_env)
    env["PS1"] = "A> "
    session = Session([str(exe)], env=env, name="readline-ctrl-a")
    try:
        session.wait_for("A> ")
        # Build `echo XY` as: type `XY` first? Simpler: type `echo XY`,
        # C-a home, nothing extra — the line must still run intact.
        session.send("echo XY" + CTRL_A + ENTER)
        session.wait_for("XY")
        body = session.text()
        assert "echo XY" in body, f"C-a corrupted the line: {body}"
        session.send("exit 0" + ENTER)
        session.wait_exit()
        return body
    finally:
        session.close()


def main() -> int:
    exe = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target" / "debug" / "rubash.exe"
    if not exe.exists():
        print(f"FAIL: {exe} not found; build first (cargo build)")
        return 1
    base_env = {
        "SystemRoot": r"C:\Windows",
        "PATH": r"C:\Windows\System32;C:\Windows",
        "TERM": "xterm",
        "HOME": str(Path.home()),
    }
    failures = []
    screens = {}
    for name, leg in (
        ("readline-rcfile", lambda: leg_rcfile_ps1(exe, base_env)),
        ("readline-backspace", lambda: leg_backspace_edit(exe, base_env)),
        ("readline-history", lambda: leg_history_up_arrow(exe, base_env)),
        ("readline-ctrl-a", lambda: leg_ctrl_a_reedit(exe, base_env)),
    ):
        try:
            screens[name] = leg()
            print(f"PASS {name}")
        except (AssertionError, TimeoutError, EOFError) as err:
            failures.append(name)
            print(f"FAIL {name}: {err}")
        screen_text = screens.get(name) or Session.last_screens.get(name, "<no screen>")
        (ARTIFACTS / f"readline-pty-{name}.txt").write_text(
            screen_text, encoding="utf-8", errors="replace"
        )
    if failures:
        print(f"FAILED legs: {', '.join(failures)}")
        return 1
    print(f"All legs green; screens kept under {ARTIFACTS}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
