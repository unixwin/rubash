//! rubash#1639 / rubash#1640 — the interactive SIGINT surface, driven
//! through a `script(1)` PTY (the only carrier with real terminal
//! signal semantics: a written ^C byte becomes a line-discipline
//! SIGINT to the foreground process group).
//!
//! #1639: Ctrl-C during a foreground wait (builtin `sleep` virtual
//! child, or an external `/usr/bin/sleep`) must discard the REST of
//! the current command line in every frame (GNU jobs.c waitchld
//! ADDINTERRUPT -> throw_to_top_level), leaving $?=130 for the next
//! command — rubash used to print `AFTER rc=130` and continue the
//! list. A TRAPPED SIGINT is the control: the trap runs when the wait
//! ends and the list continues.
//!
//! #1640: a SIGINT addressed to the shell's pid ONLY (the child is
//! not in the target set) must not abort the sleep: GNU's real
//! child never receives it and runs to completion — but GNU then
//! DISCARDS the rest of the line and reports 130 to the next one
//! (battery READJUDICATION, adj-signals §issue1640: the
//! shell-directed interrupt is remembered across the wait and
//! unwinds once the child exits; AFTER never prints, the next line
//! reads NEXT_130). The original pin's "the recorded interrupt is
//! forgotten (AFTER prints rc=0)" expectation contradicted the
//! oracle.
//!
//! Expectations verified against GNU bash 5.3.20 under the same PTY
//! discipline (pty driver probes: builtin/external/function/subshell
//! frames, `||` list, trapped control, pid-directed delivery).

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

/// The #1639 shape: send `line` (a 6s sleep followed by markers),
/// Ctrl-C mid-sleep, then probe $? from a fresh line.
fn ctrl_c_discards(line: &str, forbidden: &[&str]) {
    let mut session = spawn_session();
    session.send_line(line);
    session.pump(Duration::from_millis(1500));
    session.send_ctrl_c();
    session.pump(Duration::from_millis(1200));
    session.send_line("echo NEXT_$?");
    session.wait_for("NEXT_130", Duration::from_secs(15));
    // Assert on the OUTPUT side of the echo, not the raw transcript
    // (GATE-READJUDICATION B): the PTY necessarily echoes the typed
    // line, and every forbidden marker is part of that line's text,
    // so a bare `!transcript.contains(marker)` is false for ANY
    // product behavior. The line's own echo (its first occurrence)
    // is stripped; a marker that actually ran would print again in
    // what remains, so the check keeps its teeth. If the echo is not
    // found verbatim, fail loudly instead of passing on an
    // unstripped transcript.
    assert!(
        session.transcript.contains(line),
        "typed line's echo not found in transcript; transcript: {:?}",
        session.transcript
    );
    let output_side = session.transcript.replacen(line, "", 1);
    for marker in forbidden {
        assert!(
            !output_side.contains(marker),
            "{marker:?} ran despite the Ctrl-C discard; transcript: {:?}",
            session.transcript
        );
    }
    session.finish();
}

#[test]
fn issue1639_builtin_sleep_ctrl_c_discards_line() {
    if !script_available() {
        return;
    }
    ctrl_c_discards("sleep 6; echo AFTER1639 rc=$?", &["AFTER1639"]);
}

#[test]
fn issue1639_external_sleep_ctrl_c_discards_line() {
    if !script_available() {
        return;
    }
    ctrl_c_discards("/usr/bin/sleep 6; echo AFTER1639 rc=$?", &["AFTER1639"]);
}

#[test]
fn issue1639_function_frame_discards_whole_line() {
    if !script_available() {
        return;
    }
    ctrl_c_discards(
        "f() { sleep 6; echo IN_FUNC; }; f; echo AFTER1639",
        &["IN_FUNC", "AFTER1639"],
    );
}

#[test]
fn issue1639_subshell_frame_discards_whole_line() {
    if !script_available() {
        return;
    }
    ctrl_c_discards(
        "( sleep 6; echo IN_SUB ); echo AFTER1639",
        &["IN_SUB", "AFTER1639"],
    );
}

#[test]
fn issue1639_or_list_rhs_does_not_run() {
    if !script_available() {
        return;
    }
    // The discard outranks || semantics: the 130 is not a plain
    // failure status the list may act on.
    ctrl_c_discards("sleep 6 || echo AFTER1639", &["AFTER1639"]);
}

#[test]
fn issue1639_trapped_int_runs_trap_and_continues_line() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    session.send_line("trap 'echo TRAPPED_INT' INT; sleep 6; echo AFTER1639 rc=$?");
    session.pump(Duration::from_millis(1500));
    session.send_ctrl_c();
    // The trap fires as the interrupted wait ends; the list then
    // CONTINUES (no discard for a trapped SIGINT).
    session.wait_for("AFTER1639 rc=130", Duration::from_secs(15));
    assert!(
        session.transcript.contains("TRAPPED_INT"),
        "transcript: {:?}",
        session.transcript
    );
    session.send_line("echo NEXT_$?");
    session.wait_for("NEXT_0", Duration::from_secs(10));
    session.finish();
}

#[test]
fn issue1640_shell_directed_int_lets_builtin_sleep_complete() {
    if !script_available() {
        return;
    }
    let mut session = spawn_session();
    // Learn the shell's pid from a computed output line (the tty echo
    // shows `$$` literally; the output carries the digits).
    session.send_line("echo PIDIS_$$");
    session.pump(Duration::from_millis(800));
    let pid = session
        .transcript
        .rfind("PIDIS_")
        .and_then(|at| {
            session.transcript[at + 6..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .expect("shell pid from transcript");
    session.transcript.clear();
    session.send_line("sleep 3; echo AFTER1640 rc=$?");
    session.pump(Duration::from_millis(1000));
    // SIGINT to the shell's pid ONLY — the (virtual) child is not a
    // target, so GNU's sleep runs to completion.
    let status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("kill binary");
    assert!(status.success());
    // READJUDICATION (adj-signals §issue1640; oracle re-verified
    // under this same PTY driver): the sleep runs to completion
    // (~3s), but the rest of the line is then DISCARDED — AFTER1640
    // never prints — and the next line reports 130. GNU remembers
    // the shell-directed interrupt across the wait; it does not
    // forget it and continue the list with rc=0 as the original
    // pin expected. (rubash repaints its prompt lazily, so the
    // discard may only surface once the next line arrives.)
    session.pump(Duration::from_secs(4));
    session.send_line("echo NEXT_$?");
    session.wait_for("NEXT_130", Duration::from_secs(15));
    // AFTER1640 may appear exactly once in the transcript — the
    // tty echo of the typed line. A second occurrence would mean
    // the marker actually ran despite the discard.
    assert!(
        session.transcript.matches("AFTER1640").count() <= 1,
        "AFTER1640 ran despite the discard; transcript: {:?}",
        session.transcript
    );
    session.finish();
}
