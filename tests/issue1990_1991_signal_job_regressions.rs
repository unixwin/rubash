//! rubash#1990 / rubash#1991 regression pins (Android real-device
//! acceptance, vivo Android 15 / Termux):
//!
//! #1991(a): `kill %1` returned 0 but never delivered. Root cause: the
//! fork-child disposition restore (kill.rs pre_exec_child_dispositions,
//! the trap.c:1489 restore_original_signals port) read the LIVE kernel
//! state word for its ignore decision, so the shell's own interactive
//! SIGTERM policy ignore (sig.c:338-339 — installed for the shell only,
//! after get_original_signal snapshots the entry disposition) leaked
//! into every spawned child: background jobs started life with SIGTERM
//! at SIG_IGN and kill(2) "succeeded" into a disposition that discards
//! it. The restore now consults the entry snapshot plus the trap ''
//! record, like GNU. These pins drive an interactive shell over pipes
//! (the #1662 harness idiom: what decides delivery is the disposition
//! surface, not the tty) and assert through the shell's own job table:
//! `wait %1` reporting 128+SIGTERM / 128+SIGKILL is the delivery proof.
//!
//! #1990: on the device PTY a foreground `sleep 20` + Ctrl+C either
//! settled 130 over a still-live child or blocked until the child's
//! natural end with the interrupt forgotten ($?=0). The wait trusted
//! the terminal's process-group delivery to reach the child; rubash
//! children share the shell's group, so when the spawn arrangement
//! leaves the child outside the terminal's foreground group the
//! delivery reaches only the shell and the plain blocking waitpid had
//! no forwarding path. The armed foreground wait now polls and
//! forwards a recorded kernel SIGINT to the child once. The PTY pins
//! below reproduce the missed-delivery arrangement on Linux by
//! re-sessioning the child (`setsid`), where a terminal ^C provably
//! cannot reach it except through the shell.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

/// Interactive shell over pipes (issue1662 harness idiom).
struct InteractiveShell {
    child: Child,
    stdout: std::process::ChildStdout,
    buf: String,
}

impl InteractiveShell {
    fn spawn() -> Self {
        let mut child = Command::new(shell_bin())
            .args(["-i"])
            .env("TERM", "dumb")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn interactive rubash");
        let stdout = child.stdout.take().expect("stdout");
        Self {
            child,
            stdout,
            buf: String::new(),
        }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        stdin.write_all(line.as_bytes()).expect("write line");
        stdin.flush().expect("flush line");
    }

    /// Read until `needle` appears in the accumulated stdout or timeout.
    /// Needles are computed markers (the issue1016 idiom) so the echoed
    /// input line can never satisfy them.
    fn wait_for(&mut self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut chunk = [0u8; 4096];
        let fd = {
            use std::os::unix::io::AsRawFd;
            self.stdout.as_raw_fd()
        };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        while Instant::now() < deadline {
            if self.buf.contains(needle) {
                return true;
            }
            match self.stdout.read(&mut chunk) {
                Ok(0) => return self.buf.contains(needle),
                Ok(n) => self.buf.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        self.buf.contains(needle)
    }
}

impl Drop for InteractiveShell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn issue1991_kill_jobspec_term_and_stop_actually_deliver() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT$((6*7))\n");
    assert!(
        sh.wait_for("BOOT42", Duration::from_secs(10)),
        "boot: {}",
        sh.buf
    );

    // TERM form: the job must die by SIGTERM (wait reports 128+15).
    sh.send("sleep 30 &\n");
    sh.send("kill %1; echo K$((6*7))RC=$?\n");
    assert!(
        sh.wait_for("K42RC=0", Duration::from_secs(10)),
        "kill %1 rc: {}",
        sh.buf
    );
    sh.send("wait %1; echo W$((6*7))=$?\n");
    assert!(
        sh.wait_for("W42=143", Duration::from_secs(10)),
        "job must have died by SIGTERM (wait=143): {}",
        sh.buf
    );

    // STOP form: the job must show Stopped, then die by SIGKILL.
    sh.send("sleep 30 &\n");
    sh.send("kill -STOP %1; echo S$((6*7))RC=$?\n");
    assert!(
        sh.wait_for("S42RC=0", Duration::from_secs(10)),
        "kill -STOP rc: {}",
        sh.buf
    );
    sh.send("jobs %1; echo J$((6*7))END\n");
    assert!(
        sh.wait_for("Stopped", Duration::from_secs(10)),
        "job must be Stopped after kill -STOP %1: {}",
        sh.buf
    );
    sh.send("kill -9 %1\n");
    sh.send("wait %1; echo W$((9*9))=$?\n");
    assert!(
        sh.wait_for("W81=137", Duration::from_secs(10)),
        "job must have died by SIGKILL (wait=137): {}",
        sh.buf
    );
}

#[test]
fn issue1991_kill_unknown_jobspec_is_an_honest_error() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT$((6*7))\n");
    assert!(
        sh.wait_for("BOOT42", Duration::from_secs(10)),
        "boot: {}",
        sh.buf
    );
    // No job table entry: GNU errors with a nonzero status; the #1991
    // defect class is a fake 0, so the pin asserts the nonzero digit.
    sh.send("kill %7; echo N$((6*7))RC=$?\n");
    assert!(
        sh.wait_for("N42RC=1", Duration::from_secs(10)),
        "kill %7 with no such job must report rc=1: {}",
        sh.buf
    );
}

#[test]
fn issue1991_trap_empty_term_shields_job_but_kill9_still_works() {
    let mut sh = InteractiveShell::spawn();
    sh.send("echo BOOT$((6*7))\n");
    assert!(
        sh.wait_for("BOOT42", Duration::from_secs(10)),
        "boot: {}",
        sh.buf
    );
    // GNU trap.c:1451-1455: a signal ignored with trap '' STAYS ignored
    // in children — the fix must not overreach and strip this. The job
    // survives kill %1 (TERM) but not kill -9 %1.
    sh.send("trap '' TERM\n");
    sh.send("sleep 30 &\n");
    sh.send("kill %1; echo K$((6*7))RC=$?\n");
    assert!(
        sh.wait_for("K42RC=0", Duration::from_secs(10)),
        "kill %1 rc: {}",
        sh.buf
    );
    sh.send("jobs; echo J$((6*7))END\n");
    assert!(
        sh.wait_for("Running", Duration::from_secs(10)),
        "trap '' TERM job must still be Running after kill %1: {}",
        sh.buf
    );
    sh.send("kill -9 %1\n");
    sh.send("wait %1; echo W$((6*7))=$?\n");
    assert!(
        sh.wait_for("W42=137", Duration::from_secs(10)),
        "job must have died by SIGKILL (wait=137): {}",
        sh.buf
    );
}

fn sleep_path() -> Option<&'static str> {
    if PathBuf::from("/usr/bin/sleep").exists() {
        Some("/usr/bin/sleep")
    } else if PathBuf::from("/bin/sleep").exists() {
        Some("/bin/sleep")
    } else {
        None
    }
}

/// Drive one script(1) PTY session: run `fg_line`, send ^C once the
/// child is provably up (its pidfile exists), then — only after the
/// post-^C prompt is observed in the transcript — ask for the status
/// and the session-alive marker. Reactive pacing matters: a line
/// that reaches the PTY while the interrupted wait is still being
/// adjudicated joins the interrupted line's discard (#1639), so the
/// status question must wait for the prompt like a user would. The
/// whole session must finish far below the child's 30s natural
/// life — if the interrupt never reaches the child, the markers only
/// print after it exits on its own and the deadline kills the
/// session first.
fn pty_sigint_session(fg_line: &str, pidfile: &PathBuf, tag: &str) -> String {
    let cmdline = format!("{} -i", shell_bin().display());
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn script(1)");

    let transcript = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mut stdout = child.stdout.take().expect("child stdout");
    let reader_transcript = transcript.clone();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                    reader_transcript.lock().unwrap().push_str(&text);
                }
            }
        }
        let _ = tx.send(());
    });

    let wait_transcript = |needle: &str, timeout: Duration| -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if transcript.lock().unwrap().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        transcript.lock().unwrap().contains(needle)
    };

    let mut stdin = child.stdin.take().expect("child stdin");
    let mut drive = |line: &[u8]| {
        let _ = stdin.write_all(line);
        let _ = stdin.flush();
    };
    drive(fg_line.as_bytes());
    // The child writes its pidfile immediately before exec'ing sleep;
    // once it exists the foreground wait is certainly running.
    let child_up_deadline = Instant::now() + Duration::from_secs(8);
    while !pidfile.exists() && Instant::now() < child_up_deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(pidfile.exists(), "{tag}: fg child never came up");
    std::thread::sleep(Duration::from_millis(300));
    // ^C through the PTY: the terminal driver generates the
    // kernel-caused group SIGINT, exactly the device shape.
    drive(b"\x03");
    assert!(
        wait_transcript("^C", Duration::from_secs(8)),
        "{tag}: no ^C echo in transcript"
    );
    // Let the adjudicated wait fully land at the prompt before
    // asking for the status (see the fn comment).
    std::thread::sleep(Duration::from_millis(600));
    drive(b"echo R$((6*7))C=$?\n");
    if !wait_transcript("R42C=130", Duration::from_secs(3)) {
        // Master behavior after an external child dies by SIGINT:
        // the #1639 unwind's line discard can spend itself on the
        // first line read after the interrupt (that line never
        // executes, and $? keeps the interrupted wait's 130). Ask
        // once more — whichever branch the session took, the marker
        // now prints with the wait's own status.
        drive(b"echo R$((6*7))C=$?\n");
        let _ = wait_transcript("R42C=130", Duration::from_secs(6));
    }
    drive(b"echo A$((6*7))LIVE\n");
    let _ = wait_transcript("A42LIVE", Duration::from_secs(4));
    drive(b"exit\n");

    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(()) => {}
        Err(_) => {
            let _ = child.kill();
            panic!("{tag}: PTY session did not finish within 20s — the foreground child was not interrupted and reaped");
        }
    }
    let _ = child.wait();
    let _ = reader.join();
    let text = transcript.lock().unwrap().clone();
    text
}

fn pidfile(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rubash-{tag}-{}", std::process::id()))
}

fn assert_pid_gone(pidfile: &PathBuf, tag: &str) {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string(pidfile)
            .unwrap_or_else(|e| panic!("{tag}: pidfile missing: {e}"));
        let pid: i32 = text.trim().parse().expect("pidfile holds a pid");
        assert!(
            !PathBuf::from(format!("/proc/{pid}")).exists(),
            "{tag}: foreground child pid {pid} survived the interrupt"
        );
    }
    let _ = tag;
    let _ = std::fs::remove_file(pidfile);
}

#[test]
fn issue1990_sigint_forwarded_to_resessioned_fg_child() {
    if Command::new("script").arg("--version").output().is_err()
        || Command::new("setsid").arg("--version").output().is_err()
    {
        eprintln!("no script(1)/setsid on this host; skipping");
        return;
    }
    let Some(sleep) = sleep_path() else {
        eprintln!("no sleep binary on this host; skipping");
        return;
    };
    let pf = pidfile("issue1990-setsid");
    let _ = std::fs::remove_file(&pf);
    // The child records its own pid, then execs sleep under setsid:
    // it leads a NEW session, so the PTY's group delivery cannot
    // reach it — only the shell's forwarding can (the #1990 shape).
    let line = format!(
        "setsid sh -c 'echo $$ > {}; exec {sleep} 30'\n",
        pf.display()
    );
    let text = pty_sigint_session(&line, &pf, "issue1990-setsid");
    assert!(
        text.contains("R42C=130"),
        "interrupted wait must settle 130: {text:?}"
    );
    assert!(
        text.contains("A42LIVE"),
        "session must survive the interrupt: {text:?}"
    );
    assert_pid_gone(&pf, "issue1990-setsid");
}

#[test]
fn issue1990_sigint_healthy_fg_child_still_dies_130() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let Some(sleep) = sleep_path() else {
        eprintln!("no sleep binary on this host; skipping");
        return;
    };
    let pf = pidfile("issue1990-healthy");
    let _ = std::fs::remove_file(&pf);
    // Healthy arrangement: the child shares the terminal's foreground
    // group and dies from the group delivery itself; the wait must
    // still settle 130 with the session alive (the #1662 contract,
    // external-child form).
    let line = format!("sh -c 'echo $$ > {}; exec {sleep} 30'\n", pf.display());
    let text = pty_sigint_session(&line, &pf, "issue1990-healthy");
    assert!(
        text.contains("R42C=130"),
        "interrupted wait must settle 130: {text:?}"
    );
    assert!(
        text.contains("A42LIVE"),
        "session must survive the interrupt: {text:?}"
    );
    assert_pid_gone(&pf, "issue1990-healthy");
}
