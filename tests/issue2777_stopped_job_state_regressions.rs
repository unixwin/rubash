//! rubash#2777 — a job stopped by SIGSTOP is reported as
//! Running by `jobs`, and the exit-time job check consequently
//! says "There are running jobs." where GNU says "There are
//! stopped jobs."
//!
//! GNU's law (bash-5.3): job state is derived from the member
//! wait statuses — a job with no running member and a WIFSTOPPED
//! member is JSTOPPED (jobs.c:676 state recomputation; waitchld
//! collects stop statuses with WUNTRACED whenever job control is
//! enabled, and an interactive shell runs that reap at its reader
//! boundary, parse.y read_command's notify_and_cleanup). There
//! is no wall-clock aging: a stop the shell itself sent with
//! `kill` is learned by the next command. Consumers read the
//! same JOBSTATE: pretty_print_job for the `jobs` listing, and
//! builtins/exit.def:106-118 `exit_or_logout` — STOPPED is
//! scanned first and defers with "There are stopped jobs."
//! regardless of checkjobs; RUNNING defers with "There are
//! running jobs." only under checkjobs; the job listing prints
//! only when a deferral happens with checkjobs set.
//!
//! rubash has no waitchld/WUNTRACED poller: a stop sent by its
//! own `kill` is recorded as a DEFERRED mark (src/jobs/table.rs)
//! and published by apply_deferred_state_marks only after an
//! observation event (the shell blocked on a foreground wait) or
//! after the mark has aged 5ms past a command boundary. In the
//! deep-audit B4 session (piped interactive shell, `kill -STOP
//! $!` immediately followed by `exit`) neither condition holds,
//! so the exit check and the listing keep reading Running.
//!
//! Expectation basis: GNU bash 5.3.20's job-control semantics —
//! the B4 session shapes below were re-run against the oracle
//! under a PTY (real job control) on 2026-11-09 and produced
//! exactly the asserted texts; this is the same basis the merged
//! rubash#2397 stopped-job pins use (over pipes GNU has no job
//! control and never learns the stop, so the stopped verdicts
//! are its PTY semantics). The observation-point cells were
//! additionally verified against the oracle over pipes. The
//! script-mode control cell is plain script semantics, verified
//! byte-for-byte against the oracle.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ----------------------------------------------------------------
// Piped interactive harness (the rubash#2397 shape).

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2777-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    fs::canonicalize(&dir).expect("canonicalize sandbox")
}

/// Run one interactive pipe session feeding exactly `lines`, then
/// EOF; return (stdout, stderr, exit status).
fn run_interactive(home: &PathBuf, lines: &[&str]) -> (String, String, i32) {
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut script = lines.join("\n");
    script.push('\n');
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });
    let (tx, rx) = mpsc::channel();
    let mut out = child.stdout.take().expect("child stdout");
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (tx2, rx2) = mpsc::channel();
    let mut err = child.stderr.take().expect("child stderr");
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx2.send(buf);
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
    let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx2.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();
    let code = status.and_then(|s| s.code()).expect("session exit status");
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        code,
    )
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The deep-audit B4 session verbatim: checkjobs on, a background
/// job stopped by pid, `exit` immediately — no observation point.
/// GNU (job control): the exit defers with "There are stopped
/// jobs." and lists the job as Stopped; the second `exit` defers
/// again (the intervening echo rolled the last-builtin tracker);
/// EOF then exits with the deferral's status 1.
#[test]
fn b4_session_exit_says_stopped_jobs() {
    let home = sandbox("b4on");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &[
            "shopt -s checkjobs",
            "sleep 30 &",
            "kill -STOP $!",
            "exit",
            "echo after-exit",
            "exit",
        ],
    );
    assert_eq!(
        count(&stderr, "There are stopped jobs."),
        2,
        "both typed exits defer for the stopped job:\nstderr={stderr}"
    );
    assert!(
        !stderr.contains("There are running jobs."),
        "the stopped job must not be read as running:\nstderr={stderr}"
    );
    assert_eq!(
        count(&stdout, "Stopped"),
        2,
        "checkjobs on: each deferral lists the stopped job:\nstdout={stdout}"
    );
    assert!(
        stdout.contains("after-exit"),
        "the first exit deferred, so the echo ran:\nstdout={stdout}"
    );
    assert_eq!(code, 1, "final status is the deferral's 1 (GNU)");
}

/// The stopped-job protection does not depend on checkjobs
/// (exit.def scans STOPPED unconditionally); only the listing
/// is gated on the shopt.
#[test]
fn b4_session_checkjobs_off_still_defers_for_stopped() {
    let home = sandbox("b4off");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "shopt -u checkjobs",
            "sleep 30 &",
            "kill -STOP $!",
            "exit",
            "echo after-exit",
            "exit",
        ],
    );
    assert!(
        stderr.contains("There are stopped jobs."),
        "stopped protection applies with checkjobs off:\nstderr={stderr}"
    );
    assert!(
        stdout.contains("after-exit"),
        "the first exit deferred, so the echo ran:\nstdout={stdout}"
    );
    assert!(
        !stdout.contains("Stopped"),
        "checkjobs off: no job listing on stdout:\nstdout={stdout}"
    );
}

/// With a reap observation point (a foreground `sleep 1`, the
/// rubash#2397 device), the `jobs` LISTING itself must show the
/// stopped state — the #2397 pins asserted the exit text for
/// this session shape but never the listing.
#[test]
fn jobs_listing_shows_stopped_after_observation() {
    let home = sandbox("listing");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "set -m",
            "sleep 30 &",
            "kill -STOP %1",
            "sleep 1",
            "jobs",
            "kill -KILL %1",
            "wait %1",
            "echo W=$?",
        ],
    );
    assert!(
        stdout.contains("Stopped"),
        "the listing shows the stopped job:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("W=137"),
        "the stopped-then-killed job waits as 128+9:\nstdout={stdout}"
    );
}

/// Chain: the recorded state feeds the next step — after CONT
/// the same listing reads Running again.
#[test]
fn jobs_listing_tracks_stop_then_cont() {
    let home = sandbox("chain");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "set -m",
            "sleep 30 &",
            "kill -STOP %1",
            "sleep 1",
            "jobs > j1.txt",
            "kill -CONT %1",
            "sleep 1",
            "jobs > j2.txt",
            "kill -KILL %1",
            "wait %1",
            "cat j1.txt",
            "cat j2.txt",
        ],
    );
    let stopped_at = stdout.find("Stopped");
    let running_at = stdout.find("Running");
    assert!(
        stopped_at.is_some(),
        "first listing is Stopped:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        running_at.is_some(),
        "second listing is Running after CONT:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stopped_at < running_at,
        "the Stopped listing precedes the Running one:\nstdout={stdout}"
    );
}

/// SIGTSTP stops surface the same way as SIGSTOP stops.
#[test]
fn tstp_stop_also_shows_stopped() {
    let home = sandbox("tstp");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "set -m",
            "sleep 30 &",
            "kill -TSTP %1",
            "sleep 1",
            "jobs",
            "kill -KILL %1",
            "wait %1",
            "echo W=$?",
        ],
    );
    assert!(
        stdout.contains("Stopped"),
        "the listing shows the TSTP-stopped job:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(stdout.contains("W=137"), "stdout={stdout}");
}

// ----------------------------------------------------------------
// Script-mode control: in a non-interactive shell GNU does NOT
// learn the stop either (its waitchld runs without the job
// control reap), and `jobs` reads Running. This cell guards the
// fix against overcorrecting the script path.

fn run_script(body: &str) -> (String, String) {
    let dir = sandbox("script");
    fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn script_mode_stopped_job_reads_running_control() {
    let (stdout, stderr) = run_script(
        "sleep 30 &\nkill -STOP %1\nsleep 0.5\njobs\nkill -KILL %1\nwait %1 2>/dev/null\necho done\n",
    );
    assert!(
        stdout.contains("Running"),
        "GNU parity: script mode reads Running:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(!stdout.contains("Stopped"), "stdout={stdout}");
    assert!(stdout.ends_with("done\n"), "stdout={stdout}");
}

// ----------------------------------------------------------------
// PTY harness (the rubash#2398 shape): real job control, no
// observation point inserted — the reader boundary reap must
// surface the stop on its own. Where script(1) is unavailable
// the test returns without asserting.

mod pty {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    pub struct PtySession {
        child: std::process::Child,
        stdin: std::process::ChildStdin,
        transcript: Arc<Mutex<String>>,
    }

    impl PtySession {
        pub fn spawn() -> Option<PtySession> {
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

        pub fn send(&mut self, bytes: &str) {
            let _ = self.stdin.write_all(bytes.as_bytes());
            let _ = self.stdin.flush();
        }

        pub fn text(&self) -> String {
            self.transcript.lock().expect("transcript lock").clone()
        }

        pub fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> usize {
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

        pub fn finish(mut self) -> std::process::ExitStatus {
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
}

#[test]
fn pty_jobs_shows_stopped_without_observation() {
    let Some(mut session) = pty::PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    const LONG: Duration = Duration::from_secs(20);
    std::thread::sleep(Duration::from_millis(800));
    session.send("sleep 30 &\n");
    std::thread::sleep(Duration::from_millis(600));
    session.send("kill -STOP %1\n");
    // Two listings: the stop notice prints at most once, so two
    // Stopped sightings can only come from the listings
    // themselves (plus at most one notice).
    session.send("jobs\n");
    let first = session.wait_for("Stopped", 0, LONG);
    session.send("jobs\n");
    let _ = session.wait_for("Stopped", first, LONG);
    // Clean up: kill and reap the job, then the session exits 0.
    session.send("kill -KILL %1\n");
    session.send("wait %1\n");
    session.send("echo CLEANED42\n");
    let _ = session.wait_for("CLEANED42", 0, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {:?}", transcript);
}
