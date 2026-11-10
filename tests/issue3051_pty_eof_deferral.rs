//! Issue rubash#3051: PTY byte-level ^D exits immediately with
//! stopped jobs, bypassing the EOF deferral (pipe Ok(0) path only).
//!
//! GNU contract (bash 5.3 C source): ^D at an empty prompt is EOF as
//! the only input unit — parse.y:516-523 (`yacc_EOF` action) calls
//! `handle_eof_input_unit()` (parse.y:6973), which rolls
//! this/last with `exit_builtin` and runs the same `exit_builtin`.
//! That builtin — builtins/exit.def:100-131 `exit_or_logout` —
//! scans the live job table for STOPPED jobs (RUNNING only when
//! `checkjobs` is set), prints "There are stopped jobs." to stderr
//! (+ the listing when checkjobs is set), pins this/last to
//! exit_builtin, and returns EXECUTION_FAILURE: the exit is refused
//! and the NEXT ^D/exit is immediate (exit.def's `exit_immediate_okay`).
//!
//! Rubash's interactive reader has the same deferral —
//! `Executor::defer_interactive_eof_for_jobs` (job_builtins.rs) —
//! but only the piped/script `Ok(0)` and `Err` arms of
//! `run_interactive_stdin` (script_driver.rs) call it (#2397). The
//! byte-level ^D path — `apply_edit_char` returning
//! `EditOutcome::Eof` on `\x04` with an empty buffer, consumed by
//! the Unix tty branch, the Windows RawConsole branch, and the
//! cooked-line chars loop — sets `eof = true` directly, with no job
//! scan: the shell exits at once, silently dropping the stopped job.
//!
//! Harness shape (niu #769 lesson, same as #2849): `script(1)`
//! only allocates the pty; a parent `/usr/bin/bash -i` holds the
//! session and runs rubash as its foreground job, so rubash's pgrp
//! is not orphaned and the ^Z delivery is real. Where script(1) is
//! unavailable the PTY cells return without asserting.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct PtySession {
    child: Child,
    stdin: std::process::ChildStdin,
    transcript: Arc<Mutex<String>>,
}

impl PtySession {
    /// Spawn `script(1)` running `command` on the pty.
    fn spawn(command: &str) -> Option<PtySession> {
        if Command::new("script").arg("--version").output().is_err() {
            return None;
        }
        let home = std::env::temp_dir().join(format!("rubash3051-home-{}", std::process::id()));
        let _ = fs::create_dir_all(&home);
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

    /// Wait for whichever of the two needles lands first; returns
    /// true when `first` wins. Used for the marker probe: if the
    /// child exited, the PARENT answers `marker=unset`; if the child
    /// is still alive (the ^D was deferred), the child answers
    /// `marker=alive`.
    fn wait_for_either(&self, first: &str, second: &str, from: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let text = self.text();
                let tail = &text[from..];
                let a = tail.find(first);
                let b = tail.find(second);
                match (a, b) {
                    (Some(i), Some(j)) => return i < j,
                    (Some(_), None) => return true,
                    (None, Some(_)) => return false,
                    (None, None) => {}
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {first:?} or {second:?}; transcript so far: {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Unwind both shells: on the fixed tree the child is already
    /// gone (or is killed first by the caller), so the `exit 0`
    /// lines simply exit the parent; the second is harmless
    /// EOF-fodder.
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
/// plus the transcript offset from which the ^D sequence's output
/// must be judged.
fn stopped_sleep_session() -> Option<(PtySession, usize)> {
    let mut session = PtySession::spawn("/usr/bin/bash --norc -i")?;
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
    let probe_from = session.text().len();
    Some((session, probe_from))
}

/// Start the parent + rubash child with no jobs at all.
fn plain_session() -> Option<(PtySession, usize)> {
    let mut session = PtySession::spawn("/usr/bin/bash --norc -i")?;
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("{} -i\n", env!("CARGO_BIN_EXE_rubash")));
    std::thread::sleep(Duration::from_millis(1000));
    session.send("CHILD_MARKER=alive\n");
    std::thread::sleep(Duration::from_millis(500));
    let probe_from = session.text().len();
    Some((session, probe_from))
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The ^D cell: one ^D with a stopped job must defer — the banner
/// prints and the child stays alive to answer the marker probe.
#[test]
fn ctrl_d_once_with_stopped_job_defers() {
    let Some((mut session, probe_from)) = stopped_sleep_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("echo marker=${CHILD_MARKER-unset}\n");
    let alive = session.wait_for_either("marker=alive", "marker=unset", probe_from, LONG);
    let transcript = session.text();
    assert!(
        alive,
        "the child must survive the first ^D (deferral); transcript: {transcript:?}"
    );
    assert!(
        transcript[probe_from..].contains("There are stopped jobs."),
        "the deferral banner must print:\ntranscript: {transcript:?}"
    );
    // Unwind: kill the stopped job, then exit (the #2849 shape:
    // the death is claimed at the next read boundary, so this exit
    // is not itself deferred); a second exit is the belt-and-braces
    // immediate exit if the first one deferred on a stale entry.
    session.send("kill -KILL %1\n");
    std::thread::sleep(Duration::from_millis(250));
    session.send("exit\n");
    std::thread::sleep(Duration::from_millis(500));
    session.send("exit\n");
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

/// The second-^D cell: the deferral pins this/last to exit, so a
/// second ^D exits at once — the banner prints exactly once and the
/// parent answers the marker probe.
#[test]
fn ctrl_d_twice_second_exits() {
    let Some((mut session, probe_from)) = stopped_sleep_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("echo marker=${CHILD_MARKER-unset}\n");
    let alive = session.wait_for_either("marker=alive", "marker=unset", probe_from, LONG);
    let transcript = session.text();
    assert!(
        !alive,
        "the second ^D must exit (the first pinned exit): {transcript:?}"
    );
    assert_eq!(
        count(&transcript[probe_from..], "There are stopped jobs."),
        1,
        "exactly one deferral banner for two ^Ds:\ntranscript: {transcript:?}"
    );
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

/// Control: with no jobs at all, one ^D exits immediately and prints
/// no banner — the deferral must not fire vacuously.
#[test]
fn ctrl_d_without_stopped_job_exits() {
    let Some((mut session, probe_from)) = plain_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("echo marker=${CHILD_MARKER-unset}\n");
    let alive = session.wait_for_either("marker=alive", "marker=unset", probe_from, LONG);
    let transcript = session.text();
    assert!(!alive, "with no jobs, ^D must exit at once: {transcript:?}");
    assert!(
        !transcript[probe_from..].contains("There are"),
        "no deferral banner without jobs:\ntranscript: {transcript:?}"
    );
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

/// Pipe-EOF control (#2397's arm, already green): a stopped job
/// still defers a piped-stdin EOF exactly once, and the shell then
/// exits on the next EOF with status 0. This pins the existing
/// Ok(0) path so the byte-level fix cannot regress it.
#[test]
fn pipe_eof_with_stopped_job_still_defers() {
    let home = std::env::temp_dir().join(format!("rubash3051-pipe-{}", std::process::id()));
    let _ = fs::create_dir_all(&home);
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let mut stdin = child.stdin.take().expect("child stdin");
    let script = "set -m\nsleep 30 >/dev/null 2>&1 &\nkill -STOP %1\nsleep 1\n";
    let mut stdout = child.stdout.take().expect("child stdout");
    let mut stderr = child.stderr.take().expect("child stderr");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let t_out = std::thread::spawn(move || {
        let _ = stdout.read_to_end(&mut out_buf);
        out_buf
    });
    let t_err = std::thread::spawn(move || {
        let _ = stderr.read_to_end(&mut err_buf);
        err_buf
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let _ = writer.join();
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap()).into_owned();
    let code = status.and_then(|s| s.code()).expect("session exit status");
    assert_eq!(
        count(&stderr, "There are stopped jobs."),
        1,
        "the piped EOF defers exactly once:\nstderr={stderr}\nstdout={stdout}"
    );
    assert_eq!(code, 0, "an EOF deferral leaves the last status alone");
}
