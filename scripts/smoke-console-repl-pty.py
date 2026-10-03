#!/usr/bin/env python3
"""ConPTY probe: bare rubash.exe on a real console renders the PS1 channel.

rubash#419: a bare `rubash.exe` attached to a console used to print a
banner + a hardcoded `$ ` placeholder REPL and ignore PS1 entirely (the
wt53 themesweep had to pipe stdin through a relay to reach the real
renderer). The fix routes the console case through the same interactive
reader the piped path uses (run_interactive_stdin): the EXPANDED PS1 goes
to stderr (GNU bashline.c:460-461 `rl_outstream = stderr`), PS2 renders
on continuation reads (parse.y:2327 read_secondary_line), and the line is
echoed exactly once (the cooked console echoes each typed key; the engine
echoes again only on the non-tty path).

This probe drives the process under a real ConPTY (pywinpty + pyte — the
niubash scripts/smoke-wizard-journey.py pattern) and checks the RENDERED
SCREEN, then repeats the simple-PS1 leg against WSL GNU bash 5.3.0
(`bash --rcfile /dev/null -i`) for the shape comparison. Raw screens are
kept under target/issue-suites/results/wt64-repl/.

Exit codes: 0 pass, 1 fail, 2 skip (python deps missing).

Usage: python scripts/smoke-console-repl-pty.py [rubash.exe]
       (default binary: target/debug/rubash.exe)
"""

import sys
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
ENTER_GAP_SECONDS = 0.2
ENTER = "\r"


class Session:
    """A process under ConPTY with a pyte-parsed screen."""

    # Last screen of every session, so a failed leg still leaves its
    # evidence under the artifacts dir (keyed by a leg-supplied name).
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

    def send(self, keys):
        """Settle, then send text and Enter as separate writes (a single
        burst can lose the trailing Enter on the ConPTY bridge)."""
        time.sleep(SETTLE_SECONDS)
        if keys.endswith(ENTER) and len(keys) > 1:
            self.proc.write(keys[:-1])
            time.sleep(ENTER_GAP_SECONDS)
            self.proc.write(ENTER)
        else:
            self.proc.write(keys)


def leg_rubash_simple(exe: Path, base_env) -> str:
    """Bare rubash.exe, simple PS1: the screen must show the PS1 content."""
    env = dict(base_env)
    env["PS1"] = "X> "
    session = Session([str(exe)], env=env, name="rubash-simple")
    try:
        session.wait_for("X>")
        body = session.text()
        assert "Rubash - A Rust implementation" not in body, (
            "placeholder banner still rendered"
        )
        # One command: console echo types it once; the engine must not
        # echo it again (GNU tty readline echoes exactly once).
        session.send("echo CONSOLE-OK" + ENTER)
        session.wait_for("CONSOLE-OK")
        body = session.text()
        assert body.count("echo CONSOLE-OK") == 1, (
            f"input line echoed {body.count('echo CONSOLE-OK')} times "
            "(console echo + engine echo = double print)"
        )
        # PS2 continuation: an open `if` renders the expanded PS2 (`> `).
        session.send("if true" + ENTER)
        session.wait_for("> ")
        session.send("then" + ENTER)
        session.send("echo IF-OK" + ENTER)
        session.send("fi" + ENTER)
        session.wait_for("IF-OK")
        # eval.c:203-207: an interactive shell SURVIVES a syntax error and
        # keeps reading (a typo on the console must not kill the shell).
        session.send("echo )bad" + ENTER)
        session.send("echo SURVIVED" + ENTER)
        session.wait_for("SURVIVED")
        # exit terminates the shell (not the placeholder's "Goodbye!").
        session.send("exit 0" + ENTER)
        deadline = time.time() + 10
        while session.proc.isalive() and time.time() < deadline:
            time.sleep(0.1)
        assert not session.proc.isalive(), "shell did not exit on `exit 0`"
        return session.text()
    finally:
        session.close()


def leg_rubash_theme(exe: Path, base_env) -> str:
    """Bare rubash.exe with a theme-shaped PS1 (color + \\u@\\h + \\w +
    ignore markers): the rendered screen must show the expanded segments
    and survive a strict VT parser (no \\x01/\\x02 marker leak)."""
    env = dict(base_env)
    env["PS1"] = "\\[\\e[1;35m\\]\\u@\\h \\w\\e[0m\\n\\$ "
    session = Session([str(exe)], env=env, name="rubash-theme")
    try:
        session.send("echo THEME-OK" + ENTER)
        # pyte swallows everything after a leaked \x01, so THEME-OK
        # rendering is itself the marker-leak detector.
        session.wait_for("THEME-OK")
        body = session.text()
        assert "@" in body, f"\\u@\\h segment not expanded: {body}"
        assert "repo" in body.replace("\\", "/"), (
            f"\\w segment not expanded: {body}"
        )
        return body
    finally:
        session.close()


def leg_gnu_simple(base_env) -> str:
    """WSL GNU bash 5.3.0 on the same ConPTY with the same simple PS1:
    the reference shape for the rubash leg."""
    env = dict(base_env)
    env["PS1"] = "X> "
    session = Session(
        ["wsl.exe", "-e", "env", "PS1=X> ",
         "/usr/local/bin/bash", "--rcfile", "/dev/null", "-i"],
        env=env,
        name="gnu-simple",
    )
    try:
        session.wait_for("X>")
        session.send("echo GNU-PTY-OK" + ENTER)
        session.wait_for("GNU-PTY-OK")
        body = session.text()
        assert body.count("echo GNU-PTY-OK") == 1, (
            "GNU leg unexpectedly double-echoed; probe broken"
        )
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
        ("rubash-simple", lambda: leg_rubash_simple(exe, base_env)),
        ("rubash-theme", lambda: leg_rubash_theme(exe, base_env)),
        ("gnu-simple", lambda: leg_gnu_simple(base_env)),
    ):
        try:
            screens[name] = leg()
            print(f"PASS {name}")
        except (AssertionError, TimeoutError, EOFError) as err:
            failures.append(name)
            print(f"FAIL {name}: {err}")
        # A failed leg still leaves its last parsed screen as evidence.
        screen_text = screens.get(name) or Session.last_screens.get(name, "<no screen>")
        (ARTIFACTS / f"console-pty-{name}.txt").write_text(
            screen_text, encoding="utf-8", errors="replace"
        )
    if failures:
        print(f"FAILED legs: {', '.join(failures)}")
        return 1
    print(f"All legs green; screens kept under {ARTIFACTS}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
