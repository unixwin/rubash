//! rubash#2836(i) — an interactive shell must ignore the stop class.
//! GNU jobs.c:4949 initialize_job_signals: an interactive shell sets
//! SIGTSTP/SIGTTOU/SIGTTIN to SIG_IGN, so ^Z at an empty prompt cannot
//! stop the shell itself. The terminal delivery still reaches the
//! foreground job's own process group, so a foreground `sleep` must
//! still stop (its dispositions are restored to entry defaults in the
//! child — trap.c:1489 restore_original_signals).
//!
//! The empty-prompt pin needs the job-control-parent shape (niu #769
//! lesson): a plain `script(1)` session leaves the shell's own pgrp
//! orphaned, and the kernel discards SIGTSTP for an orphaned pgrp —
//! the pin would pass vacuously without the fix. Here `script(1)`
//! only allocates the pty; a parent `/usr/bin/bash -i` holds the
//! session and runs rubash as its foreground job, so rubash's pgrp
//! has its parent in the same session and the VSUSP delivery is real.
//! Where script(1) is unavailable the tests return without asserting.

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
    /// Spawn `script(1)` running `command` on the pty.
    fn spawn(command: &str) -> Option<PtySession> {
        if Command::new("script").arg("--version").output().is_err() {
            return None;
        }
        let home = std::env::temp_dir().join(format!("rubash2836-home-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&home);
        let mut child = Command::new("script")
            .args(["-qec", command, "/dev/null"])
            .env("TERM", "xterm")
            .env("HOME", &home)
            .env("PS1", "PARENT$ ")
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

    /// Resume-then-exit both shells: `fg` is a no-op error when the
    /// child is already at its prompt, and resumes it when ^Z stopped
    /// it — either way the two `exit` lines then unwind the stack.
    /// `exit 0` (not bare `exit`): a bare exit reuses `$?`, and the
    /// `fg` above fails when there is no job to resume.
    fn finish(mut self) -> std::process::ExitStatus {
        self.send("fg\n");
        std::thread::sleep(Duration::from_millis(300));
        self.send("exit 0\n");
        std::thread::sleep(Duration::from_millis(300));
        self.send("exit 0\n");
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
fn empty_prompt_ctrl_z_does_not_stop_the_shell() {
    let Some(mut session) = PtySession::spawn("/usr/bin/bash --norc -i") else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    // Let the parent bash settle, then start rubash as its foreground job.
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("{} -i\n", env!("CARGO_BIN_EXE_rubash")));
    std::thread::sleep(Duration::from_millis(1000));
    session.send("CHILD_MARKER=alive\n");
    std::thread::sleep(Duration::from_millis(500));
    // Offset past the assignment echo: `CHILD_MARKER=alive` itself
    // contains "MARKER=alive", so the probe needle must only match
    // output produced after this point.
    let probe_from = session.text().len();
    // ^Z at an empty prompt: VSUSP delivers SIGTSTP to the shell's own
    // (foreground) pgrp. GNU ignores it; the shell stays at its prompt.
    session.send("\x1a");
    std::thread::sleep(Duration::from_millis(800));
    // The typed line contains `OUT-$CHILD_MARKER`, never `OUT-alive`:
    // only the command's own output can produce the needle.
    session.send("echo OUT-$CHILD_MARKER\n");
    let after_out = session.wait_for("OUT-alive", probe_from, LONG);
    let _ = after_out;
    let transcript = session.text();
    let flat = transcript.replace("\r\n", "\n");
    assert!(
        !flat.contains("Stopped"),
        "the shell itself stopped on ^Z at an empty prompt: {flat:?}"
    );
    assert!(
        transcript[probe_from..].contains("OUT-alive"),
        "the echo did not run in the child shell (marker alive): {flat:?}"
    );
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}

#[test]
fn foreground_sleep_still_stops_after_the_shell_ignores_tstp() {
    // Guard for the child side of the same change: the shell's own
    // SIG_IGN for the stop class must NOT leak into its children
    // (pre_exec_child_dispositions restores SIG_DFL), so a foreground
    // `sleep` still stops on ^Z and `fg` still resumes it.
    let Some(mut session) = PtySession::spawn(&format!("{} -i", env!("CARGO_BIN_EXE_rubash")))
    else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    session.send("sleep 6\n");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("\x1a");
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    let _ = session.wait_for("sleep 6", after_notice, LONG);
    session.send("echo RC=$?\n");
    let after_rc = session.wait_for("RC=148", after_notice, LONG);
    session.send("fg\n");
    session.send("echo FGRC=$?\n");
    let after_fg = session.wait_for("FGRC=0", after_rc, LONG);
    let _ = after_fg;
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
