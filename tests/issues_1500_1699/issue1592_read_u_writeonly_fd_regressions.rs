//! rubash#1592 — `read -u N` on a WRITE-ONLY fd reported
//! `invalid file descriptor`; GNU distinguishes the two EBADF cases:
//! a descriptor that is not open at all fails read.def:378-380's
//! sh_validfd check as "invalid file descriptor", while an open but
//! unreadable descriptor fails in the read itself — read.def:911
//! "read error". Both carry `: Bad file descriptor` and status 1.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1592-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
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
fn issue1592_read_u_writeonly_fd_says_read_error() {
    let dir = scratch("writeonly");
    let (stdout, stderr, _code) = run_c_in(&dir, "exec 3>f; read -u 3 x; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("read: 3: read error: Bad file descriptor"),
        "write-only fd must be a read error, got: {stderr}"
    );
}

#[test]
fn issue1592_read_u_unopened_fd_says_invalid_file_descriptor() {
    let dir = scratch("unopened");
    let (stdout, stderr, _code) = run_c_in(&dir, "read -u 3 x; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("read: 3: invalid file descriptor: Bad file descriptor"),
        "unopened fd must be an invalid file descriptor, got: {stderr}"
    );
}
