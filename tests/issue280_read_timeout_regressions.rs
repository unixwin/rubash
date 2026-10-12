//! niubash#280: `read -t` deadlines must fire without input.
//!
//! GNU read.def arms a timeout (read.def:499-519 shtimer) and checks it at
//! the top of every read-loop iteration (read.def:659 check_read_timeout);
//! expiry returns 128+SIGALRM (read.def:553) without consuming input. The
//! console legs of this issue (fractional `-t 0.05` blocking forever on a
//! real Windows console — the ConPTY input handle stays signaled and the
//! cooked read blocks) are covered by scripts/smoke-read-tty-pty.py; these
//! tests pin the pipe-transport legs that run in CI.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Spawn `rubash -c SCRIPT` with a piped stdin the test controls.
struct Session {
    child: std::process::Child,
}

impl Session {
    fn spawn(script: &str) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("spawn rubash");
        Self { child }
    }

    fn write(&mut self, bytes: &[u8]) {
        self.child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(bytes)
            .expect("write stdin");
    }

    fn finish(mut self) -> (String, String, Option<i32>) {
        drop(self.child.stdin.take());
        let output = self.child.wait_with_output().expect("wait rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

/// Input arriving inside the deadline is read normally (GNU: select ready
/// → record served, rc 0) — the timeout must not fire spuriously.
#[test]
fn fractional_timeout_reads_ready_input() {
    let mut session = Session::spawn("read -t 5 v; echo \"rc=$? v=[$v]\"");
    std::thread::sleep(Duration::from_millis(100));
    session.write(b"hello\n");
    let (stdout, stderr, rc) = session.finish();
    assert_eq!(stdout, "rc=0 v=[hello]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Fractional seconds parse and expire: no input within 0.2s exits 142
/// promptly (read.def:553), well under the test's 10s hang guard.
#[test]
fn fractional_timeout_expires_with_142() {
    let started = Instant::now();
    let mut session = Session::spawn("read -t 0.2 v; echo \"rc=$?\"");
    std::thread::sleep(Duration::from_millis(300));
    // The deadline may already have expired and the shell exited — a broken
    // pipe here is fine, the write is best-effort input for the alive case.
    let _ = session
        .child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"late\n");
    let (stdout, stderr, rc) = session.finish();
    assert_eq!(stdout, "rc=142\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "read -t hung past its deadline"
    );
}

/// EOF transport (drained pipe): read exits EOF-shaped, never hangs.
#[test]
fn timeout_on_eof_pipe_returns_eof() {
    let (stdout, stderr, rc) = run_quiet("read -t 0.05 v; echo \"rc=$? v=[$v]\"");
    assert_eq!(stdout, "rc=1 v=[]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Integer timeouts keep working (read.def tmsec): `-t 1` on a drained
/// pipe still EOFs (non-terminal input is always readable for select).
#[test]
fn integer_timeout_still_works() {
    let (stdout, stderr, rc) = run_quiet("read -t 1 v; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

fn run_quiet(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}
