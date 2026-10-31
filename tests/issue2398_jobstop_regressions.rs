//! rubash#2398 — job control: a foreground child's SIGTSTP stop must be
//! visible to the shell. GNU runs the interactive foreground wait with
//! WUNTRACED (jobs.c waitchld) and hands the tty to the child's pgrp
//! (jobs.c give_terminal_to), so ^Z stops ONLY the child: the job is
//! marked Stopped, `$?` is 128+SIGTSTP, `jobs` lists it, and `fg`/`bg`
//! resume it. Without the channel the ^Z stops the shell's whole
//! process group and the session freezes.
//!
//! Driven through a util-linux `script(1)` PTY like the #1016/#1612
//! pins: bytes written to script's stdin land on the pty master, and
//! the slave's line discipline (ISIG) turns the ^Z byte into SIGTSTP
//! for the pty's foreground pgrp — the honest VSUSP path, no
//! shell-side key handling involved. Where script(1) is unavailable
//! the tests return without asserting.

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

#[test]
fn fg_stop_is_seen_by_jobs_and_fg_resumes_it() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    // Let the prompt settle, then run a stoppable foreground command.
    std::thread::sleep(Duration::from_millis(800));
    session.send("sleep 30\n");
    std::thread::sleep(Duration::from_millis(1200));
    // ^Z through the pty: VSUSP delivers SIGTSTP to the foreground pgrp.
    session.send("\x1a");
    // GNU (verified in a pty against bash 5.3.20): the stop notice is
    // the jobs line for the new current job, printed before the prompt.
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    let after_notice = session.wait_for("sleep 30", after_notice, LONG);
    // The wait's verdict is 128 + SIGTSTP(20) = 148.
    session.send("echo RC=$?\n");
    let after_rc = session.wait_for("RC=148", after_notice, LONG);
    // `jobs` lists the stopped job.
    session.send("jobs\n");
    let after_jobs = session.wait_for("[1]+  Stopped", after_rc, LONG);
    let after_jobs = session.wait_for("sleep 30", after_jobs, LONG);
    // `fg` prints the command text and resumes the job in the foreground.
    session.send("fg\n");
    let after_fg = session.wait_for("sleep 30", after_jobs, LONG);
    std::thread::sleep(Duration::from_millis(800));
    // ^C now reaches the resumed job's pgrp and kills it: rc 130.
    session.send("\x03");
    session.send("echo RC2=$?\n");
    let after_rc2 = session.wait_for("RC2=130", after_fg, LONG);
    // The job is gone from the table.
    session.send("jobs\n");
    session.send("echo DONE$((6*7))\n");
    let after_done = session.wait_for("DONE42", after_rc2, LONG);
    let tail = session.text();
    assert!(
        !tail[after_rc2..after_done].contains("[1]"),
        "jobs still lists the finished job: {:?}",
        &tail[after_rc2..after_done]
    );
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", session.text());
}

#[test]
fn fg_stop_then_bg_resumes_in_background() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    session.send("sleep 30\n");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("\x1a");
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    // `bg` resumes the job in the background; `jobs` shows it Running.
    session.send("bg\n");
    std::thread::sleep(Duration::from_millis(600));
    session.send("jobs\n");
    let after_jobs = session.wait_for("[1]+  Running", after_notice, LONG);
    let _ = session.wait_for("sleep 30", after_jobs, LONG);
    // Clean up: kill the job and reap it; wait reports 128+9 = 137.
    session.send("kill -9 %1\n");
    session.send("wait %1\n");
    session.send("echo W=$?\n");
    let _ = session.wait_for("W=137", after_jobs, LONG);
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", session.text());
}
