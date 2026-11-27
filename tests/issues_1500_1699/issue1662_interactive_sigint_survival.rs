//! issue#1662 regression: an untrapped Ctrl+C must not kill an interactive
//! shell. GNU installs the shell's own SIGINT handler at startup for
//! interactive shells only (sig.c:315-316 initialize_terminating_signals),
//! independent of the trap table, so a tty ^C at the prompt or during an
//! in-process builtin aborts the current unit with status 130 and the
//! shell returns to its prompt. rubash routed SIGINT purely by the trap
//! table: untrapped meant SIG_DFL everywhere outside the foreground-wait
//! guard windows, and the kernel default action killed the shell on the
//! first ^C (verified on master: tmux pane_dead_signal=2 for a single C-c
//! at an idle prompt and during `while :; do :; done`).
//!
//! These tests drive an interactive shell over pipes (no PTY): the tty
//! burst shape is covered by the PTY matrix in the #1662 investigation
//! report; a directed kill -INT exercises the same disposition surface,
//! because what decides survival is the installed handler, not the
//! delivery route. The non-interactive pin guards the other side of
//! sig.c:315-316: a script shell keeps the killing default.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

struct InteractiveShell {
    child: Child,
    stdout: std::process::ChildStdout,
    buf: String,
}

impl InteractiveShell {
    fn spawn() -> Self {
        let mut child = Command::new(shell_bin())
            .args(["-i"])
            .env("TERM", "dumb")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn interactive rubash");
        let stdout = child.stdout.take().expect("stdout");
        Self {
            child,
            stdout,
            buf: String::new(),
        }
    }

    fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        stdin.write_all(line.as_bytes()).expect("write line");
        stdin.flush().expect("flush line");
    }

    fn send_sigint(&self) {
        let rc = unsafe { libc::kill(self.pid(), libc::SIGINT) };
        assert_eq!(rc, 0, "kill -INT to shell");
    }

    /// Read until `needle` appears in the accumulated stdout or timeout.
    fn wait_for(&mut self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut chunk = [0u8; 4096];
        // Nonblocking-ish pump: set the fd nonblocking via a short poll
        // loop on read with O_NONBLOCK toggled by libc.
        let fd = {
            use std::os::unix::io::AsRawFd;
            self.stdout.as_raw_fd()
        };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        while Instant::now() < deadline {
            if self.buf.contains(needle) {
                return true;
            }
            match self.stdout.read(&mut chunk) {
                Ok(0) => return self.buf.contains(needle),
                Ok(n) => self.buf.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        self.buf.contains(needle)
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for InteractiveShell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn issue1662_sigint_at_prompt_survives_with_130() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT-1662\n");
    assert!(
        sh.wait_for("BOOT-1662", Duration::from_secs(10)),
        "shell did not reach its prompt loop: {:?}",
        sh.buf
    );
    sh.send_sigint();
    std::thread::sleep(Duration::from_millis(300));
    assert!(sh.alive(), "shell died from a prompt SIGINT");
    sh.send("echo INTRC=$?\n");
    assert!(
        sh.wait_for("INTRC=130", Duration::from_secs(10)),
        "expected $? = 130 after prompt SIGINT, output: {:?}",
        sh.buf
    );
    sh.send("echo SURVIVED-1662-PROMPT\n");
    assert!(
        sh.wait_for("SURVIVED-1662-PROMPT", Duration::from_secs(10)),
        "shell did not return to its prompt: {:?}",
        sh.buf
    );
}

#[test]
fn issue1662_sigint_during_builtin_loop_aborts_loop_not_shell() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT-1662\n");
    assert!(
        sh.wait_for("BOOT-1662", Duration::from_secs(10)),
        "shell did not reach its prompt loop: {:?}",
        sh.buf
    );
    sh.send("while :; do :; done\n");
    std::thread::sleep(Duration::from_millis(500));
    sh.send_sigint();
    std::thread::sleep(Duration::from_millis(300));
    assert!(sh.alive(), "shell died from a SIGINT during a builtin loop");
    sh.send("echo INTRC=$?\n");
    assert!(
        sh.wait_for("INTRC=130", Duration::from_secs(10)),
        "expected the loop to abort with 130, output: {:?}",
        sh.buf
    );
    sh.send("echo SURVIVED-1662-LOOP\n");
    assert!(
        sh.wait_for("SURVIVED-1662-LOOP", Duration::from_secs(10)),
        "shell did not return to its prompt: {:?}",
        sh.buf
    );
}

#[test]
fn issue1662_sigint_burst_at_prompt_survives() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT-1662\n");
    assert!(
        sh.wait_for("BOOT-1662", Duration::from_secs(10)),
        "shell did not reach its prompt loop: {:?}",
        sh.buf
    );
    for _ in 0..5 {
        sh.send_sigint();
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(sh.alive(), "shell died from a SIGINT burst at the prompt");
    sh.send("echo SURVIVED-1662-BURST\n");
    assert!(
        sh.wait_for("SURVIVED-1662-BURST", Duration::from_secs(10)),
        "shell did not return to its prompt: {:?}",
        sh.buf
    );
}

#[test]
fn issue1662_noninteractive_sigint_keeps_killing_default() {
    // The other half of sig.c:315-316: a non-interactive shell installs
    // no SIGINT handler, so an untrapped directed SIGINT still kills it
    // (the pin is interactive-only and must not widen).
    let mut child = Command::new(shell_bin())
        .args(["-c", "sleep 30"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn script shell");
    std::thread::sleep(Duration::from_millis(500));
    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    assert_eq!(rc, 0, "kill -INT to script shell");
    let status = child.wait().expect("wait for script shell");
    assert!(
        !status.success(),
        "non-interactive shell unexpectedly survived SIGINT"
    );
    // Killed by the kernel default action itself (signal 2), not by a
    // handler-converted 130 exit: the pin must not widen to scripts.
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(2),
        "expected death by SIGINT default action, got {status:?}"
    );
}
