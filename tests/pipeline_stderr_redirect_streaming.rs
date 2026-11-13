//! A pipeline's fd-2 redirects must not disable the concurrent OS-pipe
//! driver. When the concurrent admission rejected every `2>...`/`2>&1`
//! member, the pipeline fell to the sequential stage loop, which drains an
//! unbounded producer to completion before starting the consumer — so
//! `yes | head -3 2>/dev/null` never returned. GNU forks every member
//! (execute_cmd.c:2620 execute_pipeline) and applies the element's own
//! redirections inside it (redir.c do_redirections), so the producer dies
//! of SIGPIPE once `head` exits and the stderr destination is the element's
//! alone.
//!
//! Expectations measured against GNU bash 5.3.20. The 10 s watchdog only
//! bounds the regression case; a correct run finishes in milliseconds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-pipe-stderr-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Run `-c script` in `dir`, failing instead of waiting forever when the
/// pipeline regresses to the sequential drain.
fn run(s: &Scratch, script: &str) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(s.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("poll shell") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("pipeline did not terminate: {script}");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }

    let output = child.wait_with_output().expect("collect output");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn infinite_producer_terminates_with_final_stderr_null() {
    let s = Scratch::new("null");
    let (out, err, rc) = run(&s, "yes | head -3 2>/dev/null");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\ny\ny\n");
    assert_eq!(err, "");
}

#[test]
fn infinite_producer_terminates_with_final_stderr_merge() {
    let s = Scratch::new("merge");
    let (out, err, rc) = run(&s, "yes | head -3 2>&1");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\ny\ny\n");
}

#[test]
fn infinite_producer_terminates_with_final_stderr_file() {
    let s = Scratch::new("file");
    let (out, err, rc) = run(&s, "yes | head -3 2>err.txt");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\ny\ny\n");
    assert_eq!(fs::read_to_string(s.path().join("err.txt")).unwrap(), "");
}

#[test]
fn infinite_producer_terminates_with_intermediate_stderr_null() {
    let s = Scratch::new("mid");
    let (out, err, rc) = run(&s, "yes | head -3 2>/dev/null | cat");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\ny\ny\n");
}

#[test]
fn interim_stderr_file_receives_the_members_own_diagnostics() {
    let s = Scratch::new("diag");
    // `ls` writes its diagnostic to fd 2; the redirect belongs to that
    // member, so the file gets it and stdout carries only the count.
    let (out, err, rc) = run(&s, "ls /nonexistent-rubash-test 2>err.txt | wc -l");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out.trim(), "0");
    let err_file = fs::read_to_string(s.path().join("err.txt")).unwrap();
    assert!(
        err_file.contains("nonexistent-rubash-test"),
        "redirected diagnostic missing: {err_file:?}"
    );
    assert_eq!(err, "", "the member's stderr escaped its redirect");
}

#[test]
fn pipestatus_reports_the_sigpipe_shaped_producer() {
    let s = Scratch::new("ps");
    let (out, err, rc) = run(
        &s,
        "yes | head -3 2>/dev/null\necho \"st=$? ps=${PIPESTATUS[*]}\"",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\ny\ny\nst=0 ps=141 0\n");
}
