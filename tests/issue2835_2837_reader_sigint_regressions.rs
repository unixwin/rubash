//! rubash#2835 / rubash#2837 — reader-period SIGINT settlement.
//!
//! One family ("a delivery must settle at the prompt"): both are a
//! stale delivery ambushing the next unit.
//!
//! #2835: a ^C at an empty prompt dispatches and marks the empty read
//! DISCARD (trap_exec.rs:2280), but the prompt-abort path never
//! consumed the mark, so the next unit's ExitCode arm
//! (script_driver.rs run_source_impl) took the stale mark and refused
//! the exit jump — the first `exit` printed its banner and the
//! session survived. GNU settles the interrupt at the prompt: the
//! discard belongs to the aborted read, never to the next command.
//!
//! #2837: the Unix tty read loop blocked in next_key() and never
//! dispatched pending signals, so a ^C mid-line sat in the mailbox
//! until the next accepted line's first command boundary, where it
//! aborted the whole line silently. GNU readline frees the partial
//! line at delivery time (lib/readline/signals.c _rl_handle_signal,
//! SIGINT arm: rl_free_line_state()): the read aborts, the partial
//! line is dropped, $? = 130, fresh prompt, next line runs.
//!
//! Both pins drive an interactive shell over a `script(1)` PTY (the
//! only carrier with real terminal signal semantics: a written ^C
//! byte becomes a line-discipline SIGINT to the foreground process
//! group), following the #1639/#1640 harness shape. Expectations
//! verified against GNU bash 5.3.20 under the same probe
//! (r70 pty_probe.py s17/s18: GNU 2/2 PASS, rubash base 0/2).

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct PtySession {
    child: Child,
    stdin: std::process::ChildStdin,
    rx: mpsc::Receiver<Vec<u8>>,
    transcript: String,
}

fn spawn_session() -> PtySession {
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");
    let stdin = child.stdin.take().expect("child stdin");
    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut session = PtySession {
        child,
        stdin,
        rx,
        transcript: String::new(),
    };
    // Handshake on a computed marker (the tty echo of the input line
    // cannot contain it): the shell is up and executing lines.
    session.send_line("echo READY_$((30+12))");
    session.wait_for("READY_42", Duration::from_secs(30));
    session.transcript.clear();
    session
}

impl PtySession {
    fn pump(&mut self, dur: Duration) {
        let deadline = Instant::now() + dur;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match self.rx.recv_timeout(left) {
                Ok(chunk) => {
                    self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                }
                Err(_) => return,
            }
        }
    }

    fn wait_for(&mut self, marker: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !self.transcript.contains(marker) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "timed out waiting for {marker:?}; transcript: {:?}",
                self.transcript
            );
            match self.rx.recv_timeout(left) {
                Ok(chunk) => {
                    self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                }
                Err(_) => break,
            }
        }
        assert!(
            self.transcript.contains(marker),
            "marker {marker:?} never appeared; transcript: {:?}",
            self.transcript
        );
    }

    /// True once the shell process has exited (the `script` child
    /// ends with it); polls try_wait so a surviving shell fails the
    /// pin instead of hanging the test.
    fn wait_exited(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    self.pump(Duration::from_millis(100));
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return false,
            }
        }
    }

    fn send_line(&mut self, line: &str) {
        let _ = self.stdin.write_all(line.as_bytes());
        let _ = self.stdin.write_all(b"\n");
        let _ = self.stdin.flush();
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        let _ = self.stdin.write_all(bytes);
        let _ = self.stdin.flush();
    }

    fn send_ctrl_c(&mut self) {
        self.send_bytes(b"\x03");
    }

    fn shutdown(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn script_available() -> bool {
    Command::new("script").arg("--version").output().is_ok()
}

fn normalized(transcript: &str) -> String {
    transcript.replace("\r\n", "\n")
}

#[test]
fn issue2835_prompt_ctrl_c_then_exit_exits_first_time() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    // ^C at an empty prompt: the shell survives with $? = 130 ...
    session.send_ctrl_c();
    session.pump(Duration::from_millis(600));
    session.send_line("echo RC2835=$?");
    session.wait_for("RC2835=130", Duration::from_secs(15));
    // ... and the very next `exit` must actually exit. Before the
    // fix the stale discard mark swallowed this exit jump: the
    // banner printed but the session stayed alive (probe s18).
    session.send_line("exit");
    assert!(
        session.wait_exited(Duration::from_secs(10)),
        "shell survived the first `exit` after a prompt ^C; transcript: {:?}",
        session.transcript
    );
    session.shutdown();
}

#[test]
fn issue2837_partial_line_ctrl_c_next_line_runs() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    // Type half a line, then ^C: the partial line is discarded and
    // $? = 130 (GNU readline frees the line at delivery time).
    session.send_bytes(b"echo PARTIAL");
    session.pump(Duration::from_millis(600));
    session.send_ctrl_c();
    session.pump(Duration::from_millis(600));
    session.send_line("echo RC2837=$?");
    session.wait_for("RC2837=130", Duration::from_secs(15));
    // The next line runs whole. Before the fix the stale SIGINT
    // ambushed it at its first command boundary and the line
    // vanished without output (probe s17).
    session.send_line("echo AFTER-C");
    session.wait_for("AFTER-C", Duration::from_secs(15));
    session.pump(Duration::from_millis(300));
    let text = normalized(&session.transcript);
    assert!(
        text.contains("\nAFTER-C\n"),
        "AFTER-C did not run as a standalone line; transcript: {:?}",
        session.transcript
    );
    // The two typed fragments must never have been glued into one
    // accepted line: the partial line was dropped at the ^C, so the
    // next line's echo starts on a fresh prompt line.
    assert!(
        !text.contains("PARTIALecho AFTER-C"),
        "partial line survived the ^C and glued onto the next line; transcript: {:?}",
        session.transcript
    );
    session.send_line("exit");
    session.pump(Duration::from_secs(1));
    session.shutdown();
}
