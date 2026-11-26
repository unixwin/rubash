//! rubash#1593 — write redirections on `read` and on emulated external
//! commands never created/truncated their target, and an open failure
//! was silently swallowed (the command ran on with status 0).
//!
//! GNU redir.c do_redirections opens EVERY redirection of a command in
//! list order before the command runs — creating/truncating output
//! targets even on builtins whose own logic never touches them — and a
//! failed open is a redirection_error (redir.c:149-176): the command
//! does not run and the status is 1.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1593-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join("in"), "hello\n").expect("write input file");
    dir
}

fn run_c_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1593_read_output_redirect_creates_empty_file() {
    let dir = scratch("read-create");
    let (stdout, _stderr, code) = run_c_in(&dir, "read l >f1 < in; echo \"l=$l\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "l=hello\n");
    let f1 = dir.join("f1");
    assert!(f1.exists(), "read >f1 must create f1");
    assert_eq!(std::fs::read(&f1).expect("read f1").len(), 0);
}

#[test]
fn issue1593_read_stderr_redirect_creates_file() {
    let dir = scratch("read-err-create");
    let (_stdout, _stderr, code) = run_c_in(&dir, "read l 2>fe < in");
    assert_eq!(code, Some(0));
    assert!(dir.join("fe").exists(), "read 2>fe must create fe");
}

#[test]
fn issue1593_read_output_redirect_open_failure_is_fatal() {
    let dir = scratch("read-fail");
    let (stdout, stderr, _code) = run_c_in(&dir, "read l >sub/f1 < in; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("sub/f1: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
    assert!(!dir.join("sub").exists());
}

#[test]
fn issue1593_external_fd3_redirect_creates_file() {
    let dir = scratch("cat-fd3");
    let (stdout, _stderr, code) = run_c_in(&dir, "cat 3>f3 < in");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hello\n");
    assert!(dir.join("f3").exists(), "cat 3>f3 must create f3");
}

#[test]
fn issue1593_external_fd3_redirect_failure_aborts_command() {
    let dir = scratch("cat-fd3-fail");
    let (stdout, stderr, _code) = run_c_in(&dir, "cat 3>sub/fz < in; echo \"rc=$?\"");
    // The command must NOT run: its stdin content never reaches stdout.
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("sub/fz: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}
