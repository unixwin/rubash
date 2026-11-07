//! Issue rubash#2849: `kill -KILL %1` + immediate `exit` is refused —
//! the SIGKILL death is claimed one command cycle too late.
//!
//! GNU contract (bash 5.3 C source): the refusal itself is correct
//! (builtins/exit.def:100-131 `exit_or_logout` scans the live job
//! table for STOPPED jobs), but GNU's table is already current when
//! the exit builtin runs: jobs.c:4036 `sigchld_handler` calls
//! `waitchld (-1, 0)` the moment the child dies — while the shell
//! blocks in the reader — and parse.y:2533 `read_command` runs
//! `notify_and_cleanup` before the freshly read command executes.
//! So after `kill -KILL %1`, the Killed notice prints at the next
//! read boundary and the first `exit` sees no Stopped job and exits.
//!
//! Rubash learns of deaths at its boundary sweep; a death the
//! shell's own `kill` dealt was stashed past the following command
//! (the rubash#1604 fresh-fatal window, which exists so a following
//! `wait` can claim the job itself). At the interactive reader
//! boundary that stash must not hold: the reader IS GNU's recording
//! point. The fix observes job state at the top of each interactive
//! read cycle (and per group in the grouped reader for interactive
//! invocations), so the completion lands before the new line's
//! first consumer — the exit scan included.
//!
//! Harness shape (niu #769 lesson): `script(1)` only allocates the
//! pty; a parent `/usr/bin/bash -i` holds the session and runs
//! rubash as its foreground job, so rubash's pgrp is not orphaned
//! and the ^Z delivery is real. A plain pty.fork / script session
//! would leave the pgrp orphaned and the kernel would discard
//! SIGTSTP, passing vacuously. Where script(1) is unavailable the
//! tests return without asserting.
//!
//! The kill -> exit settle is short but non-zero: the defect is
//! structural (the fresh-fatal window is reader-command-counted,
//! not time-based, so the unfixed tree defers at any settle), while
//! a zero settle would race the kernel's zombie creation on the
//! fixed tree — the same physical race GNU's handler faces and
//! wins only because a human reader blocks between the lines.

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
        let home = std::env::temp_dir().join(format!("rubash2849-home-{}", std::process::id()));
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

    /// Unwind both shells: on the fixed tree the child is already
    /// gone, so the two `exit 0` lines simply exit the parent (the
    /// second is harmless EOF-fodder); on the unfixed tree the first
    /// line is the child's pinned immediate exit and the second
    /// exits the parent.
    fn finish(mut self) -> std::process::ExitStatus {
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

/// Start the parent + rubash child, set the marker in the child,
/// stop a foreground `sleep 30` with ^Z, and return the session
/// plus the transcript offset from which the kill sequence's
/// output must be judged.
fn stopped_sleep_session() -> Option<(PtySession, usize)> {
    let mut session = PtySession::spawn("/usr/bin/bash --norc -i")?;
    // Let the parent bash settle, then start rubash as its foreground job.
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("{} -i\n", env!("CARGO_BIN_EXE_rubash")));
    std::thread::sleep(Duration::from_millis(1000));
    session.send("CHILD_MARKER=alive\n");
    std::thread::sleep(Duration::from_millis(500));
    session.send("sleep 30\n");
    std::thread::sleep(Duration::from_millis(500));
    session.send("\x1a");
    let after_notice = session.wait_for("Stopped", 0, LONG);
    let _ = session.wait_for("sleep 30", after_notice, LONG);
    // Judge only output produced by the kill sequence itself: the
    // setup transcript legitimately contains "Stopped" already.
    let probe_from = session.text().len();
    Some((session, probe_from))
}

/// The marker probe discriminates whose shell answers: if the
/// child exited, the PARENT runs the echo and the marker was never
/// set there (`marker=unset`); if the child is still alive (the
/// exit was deferred), the child answers `marker=alive` and the
/// wait_for below times out on the unfixed tree.
fn assert_child_exited(session: &PtySession, probe_from: usize) {
    session.wait_for("marker=unset", probe_from, LONG);
    let flat = session
        .text()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    assert!(
        !flat.contains("There are stopped jobs."),
        "the first exit was deferred by a stale Stopped entry: {flat:?}"
    );
    assert_eq!(
        flat.matches("Killed").count(),
        1,
        "the Killed notice must print exactly once (at the read boundary): {flat:?}"
    );
}

#[test]
fn kill_kill_then_first_typed_exit_exits() {
    let Some((mut session, probe_from)) = stopped_sleep_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("kill -KILL %1\n");
    // The issue's shape: exit typed immediately after the kill (the
    // short settle only absorbs the kernel's zombie-creation race;
    // the defect is settle-independent — see the header).
    std::thread::sleep(Duration::from_millis(250));
    session.send("exit\n");
    std::thread::sleep(Duration::from_millis(1200));
    // NOTE: a second `exit` is deliberately NOT sent — on the
    // unfixed tree it would succeed and mask the deferral. The
    // marker probe below is the verdict.
    session.send("echo marker=${CHILD_MARKER-unset}\n");
    assert_child_exited(&session, probe_from);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
