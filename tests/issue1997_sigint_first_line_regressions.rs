//! rubash#1997 — after a foreground external child dies by SIGINT,
//! the FIRST line read next must execute normally (seeing the
//! recorded $?=130); only the interrupted line itself is discarded.
//!
//! The #1639 unwind mark (`fg_sigint_unwind`) was flushed at the
//! end of an input unit only when the unit ended Ok. When the unit
//! ended in Err instead — the post-command boundary poll
//! adjudicating the shell's own unconsumed SIGINT delivery into
//! ExitCode(130) after the wait had already set the mark — the
//! mark survived into the NEXT unit and its first boundary
//! converted it, silently discarding that unit's first line (no
//! execution; $? stayed 130 for the line after). GNU consumes the
//! interrupt in the list that was executing when it landed
//! (jobs.c waitchld ADDINTERRUPT -> eval.c reader DISCARD); a line
//! read afterwards can never be discarded by it.
//!
//! Driven through a `script(1)` PTY, same discipline as the
//! issue1639_1640 pins. Two foreground shapes:
//!  - `setsid /usr/bin/sleep`: the child leaves the terminal's
//!    foreground group, so the shell's wait machinery (forwarding
//!    or direct reaping, timing-dependent) ends the wait — the
//!    shape the leak was captured on (traced matrix: WAIT-OK ->
//!    LATCH-SET -> in-unit RPT-ERR -> next unit's LATCH-TOP
//!    conversion eating the line; ~1% of cycles, load-dependent).
//!  - `/usr/bin/sleep`: plain group delivery, the child and the
//!    shell are signaled together.
//! Each shape cycles several times in one session: the leak needs
//! the shell's own delivery to still be unconsumed when the wait
//! returns, a timing conjunction, so a single cycle can pass on a
//! broken tree by luck; the asserted sequence itself (FIRST_130
//! then SECOND_0 on EVERY cycle) is deterministic GNU behavior.
//!
//! Expectations verified against GNU bash 5.3.20 under the same
//! PTY discipline (FIRST_130, SECOND_0, session survives).

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

    fn send_line(&mut self, line: &str) {
        let _ = self.stdin.write_all(line.as_bytes());
        let _ = self.stdin.write_all(b"\n");
        let _ = self.stdin.flush();
    }

    fn send_ctrl_c(&mut self) {
        let _ = self.stdin.write_all(b"\x03");
        let _ = self.stdin.flush();
    }

    fn finish(mut self) {
        self.send_line("exit");
        self.pump(Duration::from_secs(2));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn script_available() -> bool {
    Command::new("script").arg("--version").output().is_ok()
}

/// One interrupt cycle: run the foreground line, Ctrl-C it, then
/// require the next two lines to execute with the GNU status
/// sequence (FIRST sees 130, SECOND sees the FIRST line's 0). A
/// discarded FIRST shows up as a missing FIRST_130 (and SECOND
/// reporting 130 instead) — the wait_for timeouts make that a
/// loud failure with the transcript attached.
fn interrupt_cycle(session: &mut PtySession, fg_line: &str, cycle: usize) {
    session.transcript.clear();
    session.send_line(fg_line);
    session.pump(Duration::from_millis(1200));
    session.send_ctrl_c();
    session.pump(Duration::from_millis(900));
    session.send_line("echo FIRST_$?");
    session.wait_for("FIRST_130", Duration::from_secs(15));
    session.send_line("echo SECOND_$?");
    session.wait_for("SECOND_0", Duration::from_secs(15));
    let _ = cycle;
}

#[test]
fn issue1997_setsid_child_sigint_first_line_executes() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    for cycle in 0..6 {
        interrupt_cycle(&mut session, "setsid /usr/bin/sleep 20", cycle);
    }
    session.finish();
}

#[test]
fn issue1997_group_child_sigint_first_line_executes() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    for cycle in 0..6 {
        interrupt_cycle(&mut session, "/usr/bin/sleep 20", cycle);
    }
    session.finish();
}
