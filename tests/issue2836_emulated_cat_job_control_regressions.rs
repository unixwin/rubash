//! rubash#2836 (r70 slice 5) — emulated external commands stand down to
//! a real spawn while the job-control channel is live. GNU runs every
//! external command as a forked child (execute_cmd.c execute_disk_command
//! -> shell_execve; jobs.c waitchld waits with WUNTRACED), so ^Z stops
//! the child alone and the shell returns to its prompt. Rubash's
//! in-process `cat`/`sed` emulations (external_file_builtins) have no
//! child process at all: ^Z lands on the shell's own process group and
//! the shell itself stops (or, once slice 1 lands Root A, the keypress
//! becomes a silent no-op while the emulation keeps reading).
//!
//! The stand-down follows the PrimarySlot::Sleep precedent
//! (command_dispatch_primary.rs, rubash#2398): when
//! `job_control_channel_active()` holds, the emulation declines and
//! the real PATH binary spawns through the job-controlled spawn path
//! (own pgrp + terminal handover + WUNTRACED wait).
//!
//! Driven through a util-linux `script(1)` PTY like the #2398 pins:
//! bytes written to script's stdin land on the pty master, and the
//! slave's line discipline (ISIG) turns the ^Z byte into SIGTSTP for
//! the pty's foreground pgrp — the honest VSUSP path. Where script(1)
//! is unavailable the tests return without asserting.

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

impl Drop for PtySession {
    fn drop(&mut self) {
        // A pin that fails mid-session (e.g. a wait_for timeout panic)
        // must not orphan the script(1) session: a stopped rubash left
        // behind holds the harness pipes open and stalls the runner.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

    /// Wait until `needle` appears at least `n` times at or after byte
    /// offset `from`. The tty line discipline echoes typed input once,
    /// so a resumed reader's own copy is the second occurrence.
    fn wait_for_count(&self, needle: &str, from: usize, n: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let text = self.text();
                let count = text[from..].matches(needle).count();
                if count >= n {
                    return text.len();
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {n}x {needle:?}; transcript so far: {:?}",
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
fn foreground_cat_stops_on_ctrl_z_and_fg_resumes_it() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    // Bare `cat` reads the tty: the foreground job is the cat child.
    session.send("cat\n");
    std::thread::sleep(Duration::from_millis(1200));
    // ^Z through the pty: VSUSP delivers SIGTSTP to the foreground pgrp.
    session.send("\x1a");
    // The stop notice names the cat job, printed before the prompt.
    // (The notice's missing leading newline is #2834, slice 1's pin —
    // this test asserts the notice, not its line shape.)
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    let after_notice = session.wait_for("cat", after_notice, LONG);
    // The wait's verdict is 128 + SIGTSTP(20) = 148.
    session.send("echo RC=$?\n");
    let after_rc = session.wait_for("RC=148", after_notice, LONG);
    // `jobs` lists the stopped cat job.
    session.send("jobs\n");
    let after_jobs = session.wait_for("[1]+  Stopped", after_rc, LONG);
    let after_jobs = session.wait_for("cat", after_jobs, LONG);
    // `fg` resumes the cat in the foreground; a line typed afterwards
    // is echoed once by the line discipline and once more by the
    // resumed cat — the second copy proves cat is reading again.
    // (fg itself prints no job line in rubash, so the test does not
    // wait for one — the input/output pair is the resumption proof.)
    session.send("fg\n");
    std::thread::sleep(Duration::from_millis(900));
    session.send("hello-after-fg\n");
    let after_line = session.wait_for_count("hello-after-fg", after_jobs, 2, LONG);
    // EOF ends the resumed cat; it exits 0.
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(600));
    session.send("echo CATRC=$?\n");
    let _ = session.wait_for("CATRC=0", after_line, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

#[test]
fn foreground_sed_stops_on_ctrl_z_and_fg_resumes_it() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    std::thread::sleep(Duration::from_millis(800));
    // The stdin-shape `sed` is the emulated face (external_sed):
    // it drains the tty in-process, so it has the same disease as cat.
    session.send("sed 's/foo/bar/'\n");
    std::thread::sleep(Duration::from_millis(1200));
    session.send("\x1a");
    let after_notice = session.wait_for("[1]+  Stopped", 0, LONG);
    let after_notice = session.wait_for("sed", after_notice, LONG);
    session.send("echo RC=$?\n");
    let after_rc = session.wait_for("RC=148", after_notice, LONG);
    session.send("jobs\n");
    let after_jobs = session.wait_for("[1]+  Stopped", after_rc, LONG);
    let after_jobs = session.wait_for("sed", after_jobs, LONG);
    // `fg` resumes sed; the transformed line proves it is reading
    // again (the tty echo shows `foo-two`, sed's output `bar-two`).
    // (fg itself prints no job line in rubash, so the test does not
    // wait for one — the input/output pair is the resumption proof.)
    session.send("fg\n");
    std::thread::sleep(Duration::from_millis(900));
    session.send("foo-two\n");
    let after_line = session.wait_for("bar-two", after_jobs, LONG);
    session.send("\x04");
    std::thread::sleep(Duration::from_millis(600));
    session.send("echo SEDRC=$?\n");
    let _ = session.wait_for("SEDRC=0", after_line, LONG);
    let transcript = session.text();
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

#[test]
fn foreground_cat_file_operands_still_work_under_job_control() {
    let Some(mut session) = PtySession::spawn() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    // Standing down routes `cat file` to the real binary too; the
    // output must be byte-identical to the file's contents.
    let path = std::env::temp_dir().join(format!("r70s5-cat-{}.txt", std::process::id()));
    std::fs::write(&path, "alpha\nbeta\n").expect("write probe file");
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("cat {}\n", path.display()));
    let after_cat = session.wait_for("alpha", 0, LONG);
    let after_cat = session.wait_for("beta", after_cat, LONG);
    session.send("echo CATFILE=$?\n");
    let _ = session.wait_for("CATFILE=0", after_cat, LONG);
    let transcript = session.text();
    let status = session.finish();
    let _ = std::fs::remove_file(&path);
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}
