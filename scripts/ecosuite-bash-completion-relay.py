#!/usr/bin/env python3
"""rubash#2905 pexpect bridge relay: rubash TTY line-editor byte normalizer.

Problem (measured in the eco-2905 lane, see issue #2905):
  rubash's interactive line editor prints every prompt twice in the byte
  stream -- the expanded prompt, then an empty-buffer repaint frame
  ``\\r<prompt><buf>ESC[K\\rESC[<col>C`` (console_readline::redraw_line),
  and repaints after every keystroke. The bash-completion pytest suite
  (conftest.py) syncs on GNU readline bytes -- prompt once, plain tty echo,
  ``echo $?`` status probe anchored at line start -- so unmodified it
  errors every test at fixture setup (status "unknown") and TAB never
  reaches a completion listing the suite can read.

What this relay does (transport only, no completion semantics):
  * Spawns the real shell on its own pty (24x240, like conftest) and
    forwards the harness's input bytes to it unchanged.
  * Passes the shell's output through verbatim EXCEPT for repaint frames,
    which it converts to plain incremental echo: for a frame carrying buffer
    ``B`` it emits only ``B`` past what was already emitted for this line.
    The prompt itself is emitted once (the shell's own print); the frame's
    duplicate prompt and its ESC[K / cursor-position bytes are dropped.
  * Anything that is not a repaint frame (command output, completion
    listings, errors, prompts) passes through byte-for-byte.

What it never does: it does not invent, suppress, or reorder completion
results; it does not touch the suite, its assertions, or expected values;
it does not modify the shell. If the shell completes nothing, the suite
sees nothing -- exactly as the shell behaved.

Usage (conftest spawns ``$BASH_COMPLETION_TEST_BASH --norc``):
    BASH_COMPLETION_TEST_BASH="/path/ecosuite-bash-completion-relay.py /path/to/rubash"
    -> argv: relay.py /path/to/rubash --norc
Prompt tail is taken from env RELAY_PROMPT_TAIL (default "/@", the
suite's PS1). Only frames whose prompt tail matches are normalized.
"""

import fcntl
import os
import re
import struct
import sys
import termios
import threading

PROMPT_TAIL = os.environ.get("RELAY_PROMPT_TAIL", "/@")
# Frame: CR prompt-tail buffer ESC[K CR ESC[<col>C
FRAME_RE = re.compile(
    rb"\r" + re.escape(PROMPT_TAIL.encode()) + rb"(.*?)\x1b\[K\r\x1b\[(\d+)C",
    re.DOTALL,
)


def set_winsize(fd, rows=24, cols=240):
    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    except OSError:
        pass


def main(argv):
    if len(argv) < 2:
        sys.stderr.write("usage: relay.py <shell> [args...]\n")
        return 2
    shell_argv = argv[1:]

    # The suite's pty is our stdio. Take it raw so the kernel neither
    # echoes the harness's bytes back nor turns ^C into SIGINT: both must
    # reach the shell as input bytes, exactly as with a real shell spawn.
    stdin_fd = sys.stdin.fileno()
    stdout_fd = sys.stdout.fileno()
    try:
        old_attr = termios.tcgetattr(stdin_fd)
        raw_attr = termios.tcgetattr(stdin_fd)
        # cfmakeraw equivalent
        raw_attr[0] &= ~(termios.BRKINT | termios.ICRNL | termios.INPCK
                         | termios.ISTRIP | termios.IXON)
        raw_attr[1] &= ~termios.OPOST
        raw_attr[2] |= termios.CS8
        raw_attr[3] &= ~(termios.ECHO | termios.ICANON | termios.IEXTEN
                         | termios.ISIG)
        raw_attr[6][termios.VMIN] = 1
        raw_attr[6][termios.VTIME] = 0
        termios.tcsetattr(stdin_fd, termios.TCSANOW, raw_attr)
    except (termios.error, OSError):
        old_attr = None

    import pty as pty_mod

    pid, master_fd = pty_mod.fork()
    if pid == 0:
        # Child: the shell, stdio on the pty slave.
        try:
            os.execvp(shell_argv[0], shell_argv)
        except OSError as e:
            sys.stderr.write("relay: exec %s failed: %s\n" % (shell_argv[0], e))
            os._exit(127)
    set_winsize(master_fd)
    # Forward SIGWINCH-less: size is fixed by the suite (24x240).

    stop = threading.Event()

    def pump_in():
        # Harness -> shell, byte-for-byte.
        try:
            while not stop.is_set():
                try:
                    data = os.read(stdin_fd, 4096)
                except OSError:
                    break
                if not data:
                    break
                try:
                    os.write(master_fd, data)
                except OSError:
                    break
        finally:
            stop.set()
            try:
                os.close(master_fd)
            except OSError:
                pass

    def pump_out():
        # Shell -> harness, with repaint frames normalized to plain echo.
        pending = b""
        emitted = b""  # buffer text already emitted for the current line
        out = os.fdopen(os.dup(stdout_fd), "wb", buffering=0)

        def flush_tail():
            # Child is gone: anything still held back goes out verbatim.
            nonlocal pending
            if pending:
                out.write(pending)
                pending = b""

        try:
            while not stop.is_set():
                try:
                    chunk = os.read(master_fd, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                pending += chunk
                # Extract leading frames / passthrough bytes.
                while pending:
                    m = FRAME_RE.match(pending)
                    if m and m.start() == 0:
                        buf = m.group(1)
                        col = int(m.group(2))
                        # Sanity: col must equal prompt width + buffer width
                        # for this to be a genuine repaint frame.
                        if col == len(PROMPT_TAIL.encode()) + len(buf):
                            if buf == b"":
                                # Fresh line (^C abort / new prompt): the
                                # buffer is empty; emit nothing, reset.
                                emitted = b""
                            elif buf.startswith(emitted):
                                out.write(buf[len(emitted):])
                                emitted = buf
                            elif emitted.startswith(buf):
                                # Deletion: backspace over the removed tail.
                                # (Suite inputs do not delete mid-line except
                                # via ^C, which resets through an empty
                                # frame; kept for byte honesty.)
                                out.write(b"\x08" * (len(emitted) - len(buf)))
                                emitted = buf
                            else:
                                # Replacement: re-emit from common prefix.
                                n = 0
                                while (n < min(len(emitted), len(buf))
                                       and emitted[n] == buf[n]):
                                    n += 1
                                out.write(
                                    b"\x08" * (len(emitted) - n) + buf[n:]
                                )
                                emitted = buf
                            pending = pending[m.end():]
                            continue
                        # Not a genuine frame (col mismatch): fall through
                        # and pass the bytes verbatim.
                    # No frame at head: pass through up to the next frame
                    # start, or the whole pending chunk if none.
                    m2 = FRAME_RE.search(pending)
                    if m2 is None:
                        # Hold back a trailing fragment that may be the
                        # start of a frame split across reads: a lone \r
                        # whose following bytes are a prefix of, or start
                        # with, the prompt tail.
                        cut = len(pending)
                        idx = pending.rfind(b"\r")
                        if idx != -1:
                            after = pending[idx + 1:]
                            tail = PROMPT_TAIL.encode()
                            if after == tail[:len(after)] or (
                                after.startswith(tail)
                            ):
                                cut = idx
                        head = pending[:cut]
                        out.write(head)
                        if b"\n" in head:
                            emitted = b""
                        pending = pending[cut:]
                        break
                    head = pending[:m2.start()]
                    out.write(head)
                    if b"\n" in head:
                        emitted = b""
                    pending = pending[m2.start():]
                    # Loop: the frame at head is handled next iteration.
                    # If it is not a genuine frame, the branch above falls
                    # through; guard against a spin by emitting one byte.
                    if pending.startswith(b"\r" + PROMPT_TAIL.encode()):
                        mm = FRAME_RE.match(pending)
                        if mm and int(mm.group(2)) != (
                            len(PROMPT_TAIL.encode()) + len(mm.group(1))
                        ):
                            out.write(pending[:1])
                            pending = pending[1:]
        finally:
            flush_tail()
            stop.set()
            try:
                out.close()
            except OSError:
                pass

    t_in = threading.Thread(target=pump_in, daemon=True)
    t_out = threading.Thread(target=pump_out, daemon=True)
    t_in.start()
    t_out.start()
    _, status = os.waitpid(pid, 0)
    stop.set()
    try:
        os.close(master_fd)
    except OSError:
        pass
    if old_attr is not None:
        try:
            termios.tcsetattr(stdin_fd, termios.TCSANOW, old_attr)
        except (termios.error, OSError):
            pass
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    return 128 + (os.WTERMSIG(status) if os.WIFSIGNALED(status) else 0)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
