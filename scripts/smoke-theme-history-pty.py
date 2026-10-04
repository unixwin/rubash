#!/usr/bin/env python3
"""ConPTY regression pin: the bash-it theme history shape (wt90/themehang).

Owner symptom this pins: sourcing a bash-it theme (codeword / gitline) in
an interactive session froze or killed the session. Two engine defects
were live in that shape:

1. The PROMPT_COMMAND runner ran on a FRESH SessionHistory and recorded
   its own text, so every prompt appended the runner into $HISTFILE via
   the theme's `history -a` and DROPPED the commands typed since the last
   prompt (GNU: eval.c:305 execute_prompt_command -> parse_and_execute in
   the current shell, never recorded).
2. `${var?}`-class word-expansion errors exited the whole interactive
   session (GNU: subst.c:10416-10418 / 11032-11034 / 8949 + expr.c
   :1208-1216 take the DISCARD branch when interactive_shell is set;
   eval.c:111-128 keeps reader_loop alive).

This probe drives the real interactive binary under a real ConPTY
(pywinpty + pyte, the scripts/smoke-console-repl-pty.py pattern) with the
bash-it lib pieces verbatim (lib/history.bash auto-save/auto-load +
themes/base.theme.bash _save-and-reload-history + lib/preexec.bash
safe_append_prompt_command, string branch) around the real codeword
theme, HISTCONTROL=auto, then presses ENTER twice: each ENTER fires
PROMPT_COMMAND (history -a && history -c && history -r). The session
passes when both prompt tags render within budget AND the HISTFILE after
exit holds the user command exactly once and zero runner copies.

Exit codes: 0 pass, 1 fail, 2 skip (python deps missing).

Usage: python scripts/smoke-theme-history-pty.py [rubash.exe]
       (default binary: target/debug/rubash.exe)
"""

import re
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
ARTIFACTS = REPO / "target" / "issue-suites" / "results" / "wt90-themehang"
ARTIFACTS.mkdir(parents=True, exist_ok=True)

COLS, ROWS = 100, 30
ENTER = "\r"
PROMPT_BUDGET_SECONDS = 5.0
# The owner-reported hang means no next prompt for 15s+; the budget above
# is 3x tighter than the hang definition while leaving CI slack.

# The bash-it pieces, verbatim from the clone the captain fetched
# (target/bit-real): lib/preexec.bash safe_append_prompt_command (the
# PROMPT_COMMAND-string branch — bash-preexec not loaded), lib/history.bash
# _bash-it-history-auto-save/_bash-it-history-auto-load,
# themes/base.theme.bash:648 _save-and-reload-history, and the color names
# codeword.theme.bash interpolates through ${name?}.
HARNESS = r"""
normal=$'\033[0m'
bold_red=$'\033[1;31m'
bold_green=$'\033[1;32m'
green=$'\033[32m'
blue=$'\033[34m'
yellow=$'\033[33m'

function safe_append_prompt_command() {
	local prompt_re prompt_er

	prompt_re='(^|[^[:alnum:]_])'
	prompt_er='([^[:alnum:]_]|$)'
	if [[ ${PROMPT_COMMAND} =~ ${prompt_re}"${1}"${prompt_er} ]]; then
		return
	elif [[ -z ${PROMPT_COMMAND} ]]; then
		PROMPT_COMMAND="${1}"
	else
		PROMPT_COMMAND="${1};${PROMPT_COMMAND}"
	fi
}

function _bash-it-history-auto-save() {
	case $HISTCONTROL in
		*'noauto'* | *'autoload'*)
			:
			;;
		*'auto'*)
			history -a
			;;
		*)
			shopt -q histappend && history -a && return
			;;
	esac
}

function _bash-it-history-auto-load() {
	case $HISTCONTROL in
		*'noauto'*)
			:
			;;
		*'autosave'*)
			history -a
			;;
		*'autoloadnew'*)
			history -n
			;;
		*'auto'*)
			history -a && history -c && history -r
			;;
		*)
			:
			;;
	esac
}

function _save-and-reload-history() {
	local autosave="${1:-${HISTORY_AUTOSAVE:-0}}"
	[[ ${autosave} -eq 1 ]] && local HISTCONTROL="${HISTCONTROL:-}${HISTCONTROL:+:}autoshare"
	_bash-it-history-auto-save && _bash-it-history-auto-load
}

function scm_prompt() { :; }
function virtualenv_prompt() { :; }

source "@CODEWORD_THEME@"

function __probe_ps1_tag() {
	PROBE_N=$(( ${PROBE_N:-0} + 1 ))
	PS1="${PS1}[${PROBE_N}]"
}
PROMPT_COMMAND="${PROMPT_COMMAND};__probe_ps1_tag"
"""


class Session:
    def __init__(self, argv, env):
        self.proc = PtyProcess.spawn(
            argv, cwd=str(REPO), env=env, dimensions=(ROWS, COLS)
        )
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        import threading

        self._reader = threading.Thread(target=self._pump, daemon=True)
        self._reader.start()

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

    def max_tag(self):
        tags = re.findall(r"\[(\d+)\]", self.text())
        return max((int(t) for t in tags), default=0)

    def send_line(self, line):
        time.sleep(0.4)
        self.proc.write(line)
        time.sleep(0.2)
        self.proc.write(ENTER)

    def wait_tag_at_least(self, n, what):
        deadline = time.time() + PROMPT_BUDGET_SECONDS
        while time.time() < deadline:
            if self.max_tag() >= n:
                return time.time()
            if not self.proc.isalive():
                raise RuntimeError(f"session DIED waiting {what}:\n{self.text()}")
            time.sleep(0.02)
        raise TimeoutError(
            f"HANG: no prompt tag >= {n} within {PROMPT_BUDGET_SECONDS}s "
            f"({what}); screen:\n{self.text()}"
        )


def main() -> int:
    exe = Path(
        sys.argv[1] if len(sys.argv) > 1 else REPO / "target" / "debug" / "rubash.exe"
    )
    if not exe.exists():
        print(f"FAIL: {exe} not found; build first (cargo build)")
        return 1
    theme = Path("D:/repo/rubash/target/codeword.theme.bash")
    if not theme.exists():
        print(f"SKIP: {theme} not found (bash-it fixture)")
        return 2

    work = Path(tempfile.mkdtemp(prefix="wt90-pin-"))
    harness = work / "theme-harness.sh"
    harness.write_text(
        HARNESS.replace("@CODEWORD_THEME@", theme.as_posix()),
        encoding="utf-8",
        newline="\n",
    )
    histfile = work / "hist.txt"
    histfile.write_text(
        "echo seed-1\necho seed-2\necho seed-3\n", encoding="utf-8", newline="\n"
    )
    before = histfile.read_text()

    rcfile = work / "rc.sh"
    rcfile.write_text(
        "export HISTCONTROL=auto\n"
        f"export HISTFILE={histfile.as_posix()}\n"
        f"source {harness.as_posix()}\n",
        encoding="utf-8",
        newline="\n",
    )
    env = {
        "SystemRoot": r"C:\Windows",
        "PATH": r"C:\Windows\System32;C:\Windows",
        "TERM": "xterm",
        "HOME": str(work),
    }
    try:
        session = Session([str(exe), "--rcfile", str(rcfile)], env)
        session.wait_tag_at_least(1, "first themed prompt")
        session.send_line("echo PIN-MARK")
        deadline = time.time() + PROMPT_BUDGET_SECONDS
        while time.time() < deadline and "PIN-MARK" not in session.text():
            if not session.proc.isalive():
                raise RuntimeError("session died before PIN-MARK rendered")
            time.sleep(0.02)
        if "PIN-MARK" not in session.text():
            raise TimeoutError("PIN-MARK never rendered")
        # The owner shape: pure ENTERs, each firing PROMPT_COMMAND.
        t_enter = time.time()
        session.proc.write(ENTER)
        session.wait_tag_at_least(3, "first bare ENTER")
        first = time.time() - t_enter
        t_enter = time.time()
        session.proc.write(ENTER)
        session.wait_tag_at_least(4, "second bare ENTER")
        second = time.time() - t_enter
        session.proc.write("exit\r")
        for _ in range(50):
            if not session.proc.isalive():
                break
            time.sleep(0.1)
        after = histfile.read_text()
    finally:
        try:
            session.proc.terminate(force=True)
        except Exception:
            pass
        (ARTIFACTS / "smoke-theme-history-pty-screen.txt").write_text(
            session.text(), encoding="utf-8", errors="replace"
        )

    failures = []
    if first > PROMPT_BUDGET_SECONDS or second > PROMPT_BUDGET_SECONDS:
        failures.append(f"prompt latency over budget: {first:.2f}s / {second:.2f}s")
    if after.count("echo PIN-MARK") != 1:
        failures.append(
            f"PIN-MARK must be written exactly once, got {after.count('echo PIN-MARK')}"
        )
    runner_marks = [m for m in ("__rubash_pc", "PROMPT_COMMAND") if m in after]
    if runner_marks:
        failures.append(f"runner text leaked into HISTFILE: {runner_marks}")
    for seed_line in ("echo seed-1", "echo seed-2", "echo seed-3"):
        if seed_line not in after:
            failures.append(f"seed line lost from HISTFILE: {seed_line}")

    if failures:
        print(f"FAIL theme-history-pty: {'; '.join(failures)}")
        return 1
    print(
        f"PASS theme-history-pty: ENTER latencies {first:.2f}s/{second:.2f}s, "
        "HISTFILE clean (user line once, no runner copies, seeds intact)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
