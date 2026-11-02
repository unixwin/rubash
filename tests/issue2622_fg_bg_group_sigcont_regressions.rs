//! rubash#2622 — fg/bg must resume a stopped job by signalling its
//! process GROUP, not the shell's recorded pid list. GNU continue_job
//! (jobs.c:3926-3928) delivers the resume SIGCONT with
//! `killpg (jobs[job]->pgrp, SIGCONT)`: the group also holds children
//! the job's leader forked after spawn, which appear in no pid list
//! the shell keeps. Per-pid delivery resumes the leader alone and the
//! job wedges forever — the leader waits on children that stay
//! stopped.
//!
//! The decisive shape is a leader that forks an unrecorded child:
//! `sh -c 'sleep 3; echo …'` — the job table records only the `sh`
//! pid; the `sleep` grandchild lives in the job's pgrp but in no
//! recorded list. Driven through a util-linux `script(1)` PTY like
//! the #2398 pins: ^Z through the pty stops the whole foreground
//! pgrp (leader AND grandchild), then fg/bg must bring BOTH back.
//! Where script(1) is unavailable the tests return without asserting.
//!
//! End-to-end acceptance on the host side: niubash PR #862's
//! pty_gate_s8 (Ctrl-Z/fg over the real terminal path) exercises this
//! same resume through the niu binary once niu's rubash pin rises to
//! a build containing this fix.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct PtySession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    transcript: Arc<Mutex<String>>,
}

impl PtySession {
    fn spawn() -> Option<PtySession> {
        if Command::new("script").arg("--version").output().is_err() {
            return None;
        }
        let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
        let mut child = Command::new("script")
            .args(["-qec", &cmdline, "/dev/null"])
            .env("TERM", "xterm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn script(1)");
        let stdin = child.stdin.take().expect("script stdin");
        let mut stdout = child.stdout.take().expect("script stdout");
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&transcript);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        sink.lock().expect("transcript lock").push_str(&text);
                    }
                }
            }
        });
        Some(PtySession {
            child,
            stdin,
            transcript,
        })
    }

    fn send(&mut self, bytes: &str) {
        let _ = self.stdin.write_all(bytes.as_bytes());
        let _ = self.stdin.flush();
    }

    fn text(&self) -> String {
        self.transcript.lock().expect("transcript lock").clone()
    }

    /// Wait until `needle` appears in the transcript at or after byte
    /// offset `from`. Returns the offset just past the match.
    fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let text = self.text();
                if let Some(pos) = text[from..].find(needle) {
                    return from + pos + needle.len();
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; transcript so far: {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn finish(mut self) -> std::process::ExitStatus {
        self.send("exit\n");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait().expect("poll script(1)") {
                Some(status) => return status,
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "session did not exit; transcript: {:?}",
                        self.text()
                    );
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

const LONG: Duration = Duration::from_secs(20);

/// Start `sh -c 'sleep 3; echo <marker>'` in the foreground and stop
/// it with ^Z once the leader has forked its `sleep`. Returns the
/// transcript offset just past the stop notice. The markers are
/// searched with a leading "\n" so the match is the job's own output
/// line — the typed command line, the stop notice, and fg's reprint
/// of the command text all carry the marker mid-line (after
/// `echo `), never at a line start.
fn stopped_leader_with_child(session: &mut PtySession, marker: &str) -> usize {
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("sh -c 'sleep 3; echo {marker}'\n"));
    std::thread::sleep(Duration::from_millis(1200));
    // ^Z through the pty: VSUSP delivers SIGTSTP to the foreground
    // pgrp — the leader AND its unrecorded sleep child both stop.
    session.send("\x1a");
    session.wait_for("[1]+  Stopped", 0, LONG)
}

#[test]
fn fg_resumes_unrecorded_children_of_stopped_job() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    let after_notice = stopped_leader_with_child(&mut session, "INNER-FG-DONE");
    // GNU: fg's killpg SIGCONT resumes the whole group, the sleep
    // finishes, the leader prints and exits 0. Per-pid delivery
    // resumes only the leader; its sleep stays stopped and fg never
    // returns — the marker line never appears.
    session.send("fg\n");
    let after_marker = session.wait_for("\nINNER-FG-DONE", after_notice, LONG);
    session.send("echo FGRC=$?\n");
    let _ = session.wait_for("FGRC=0", after_marker, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}

#[test]
fn bg_resumes_unrecorded_children_of_stopped_job() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    let after_notice = stopped_leader_with_child(&mut session, "INNER-BG-DONE");
    // GNU: bg's killpg SIGCONT resumes the whole group in the
    // background; the leader's echo lands on the terminal once the
    // sleep finishes. Per-pid delivery leaves the sleep stopped, so
    // the echo never comes and the job never completes.
    session.send("bg\n");
    let after_marker = session.wait_for("\nINNER-BG-DONE", after_notice, LONG);
    session.send("wait %1\n");
    session.send("echo W=$?\n");
    let _ = session.wait_for("W=0", after_marker, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
