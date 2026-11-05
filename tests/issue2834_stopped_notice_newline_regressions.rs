//! rubash#2834 — the Stopped notice needs GNU's leading newline.
//! GNU jobs.c:4690-4691 notify_of_job_status prints `fprintf (stderr,
//! "\n")` in its JSTOPPED arm before pretty_print_job, so the echoed
//! ^Z sits on its own line (`^Z\n[1]+  Stopped`). Without it the
//! notice glues onto the echo (`^Z[1]+  Stopped`).
//!
//! Driven through a util-linux `script(1)` PTY like the #2398 pins:
//! bytes written to script's stdin land on the pty master, and the
//! slave's line discipline (ISIG) turns the ^Z byte into SIGTSTP for
//! the pty's foreground pgrp — the honest VSUSP path. Where script(1)
//! is unavailable the test returns without asserting.

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
            .env("HOME", std::env::temp_dir().join("rubash2834-home"))
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

impl Drop for PtySession {
    fn drop(&mut self) {
        // Never leak a stopped session behind a failed assert.
        let _ = self.child.kill();
    }
}

const LONG: Duration = Duration::from_secs(20);

#[test]
fn stopped_notice_starts_on_its_own_line() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    session.send("sleep 6\n");
    std::thread::sleep(Duration::from_millis(1200));
    // ^Z through the pty: VSUSP delivers SIGTSTP to the foreground pgrp.
    session.send("\x1a");
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    let _ = session.wait_for("sleep 6", after_notice, LONG);
    // The notice must start on its own line: the echoed ^Z and the
    // `[1]+` must be separated by a newline (pty bytes are \r\n).
    let transcript = session.text();
    let flat = transcript.replace("\r\n", "\n");
    assert!(
        flat.contains("^Z\n[1]+  Stopped"),
        "Stopped notice glued onto the ^Z echo: {flat:?}"
    );
    assert!(
        !flat.contains("^Z[1]+"),
        "Stopped notice glued onto the ^Z echo: {flat:?}"
    );
    // The stopped job still resumes and completes via fg.
    session.send("fg\n");
    session.send("echo FGRC=$?\n");
    let after_fg = session.wait_for("FGRC=0", after_notice, LONG);
    let _ = after_fg;
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
