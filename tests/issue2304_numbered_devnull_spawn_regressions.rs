//! rubash#2304 — numbered redirects targeting /dev/null (`3</dev/null`,
//! `3>/dev/null`, `3<>/dev/null`) on an external command were ALL
//! skipped by the spawn-delivery walk (apply_external_fd_delivery's
//! `is_null_device` arm, inherited from the #180 guard bundle), so the
//! spawned child read EBADF at the slot. GNU do_redirection_internal
//! hands every file target to redir_open (redir.c:949) and /dev/null
//! is not one of redir.c's special filenames, so it takes the plain
//! open with the kind's flags (redir.c:745; `<>` is O_RDWR|O_CREAT,
//! make_cmd.c:682) and the descriptor reaches the child like any
//! file's.
//!
//! The fix drops the null arm from the walk's skip: all three open
//! ladders below the skip own the null class explicitly and return a
//! real descriptor of the mapped device (open_classified_input's null
//! arm, create_redirect_output's null arm,
//! probe_input_redirect_readwrite's NullDevice fall-through).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the cluster probe table (probes/probe-all.sh): every cell below is
//! red on master 1cdace4c (inner_rc=2, "Bad file descriptor") and
//! green with the fix; the stdio controls are green on both.
//!
//! OBSERVER NOTE: observers are a foreign `sh` and the rubash binary
//! itself re-invoked — never a `./reader.sh` script: rubash runs
//! ENOEXEC scripts in-process, where spawn delivery does not appear
//! (rubash#2270/#2286).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Fixture {
    root: PathBuf,
    work: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2304_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    std::fs::create_dir_all(&work).expect("create fixture work dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let work = root.join("work");
    Fixture { root, work }
}

impl Fixture {
    fn run(&self, script: &str) -> (String, String, Option<i32>) {
        let script_path = self.root.join("probe.sh");
        std::fs::write(&script_path, script).expect("write probe script");
        let child = Command::new(RUBASH)
            .arg(&script_path)
            .env("RUBASH_UNDER_TEST", RUBASH)
            .current_dir(&self.work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }
}

/// Input kind: the child must read EOF from the delivered slot, not
/// EBADF. The issue's first reproducer.
#[cfg(unix)]
#[test]
fn issue2304_devnull_input_foreign_child() {
    let fx = fixture("input");
    let (stdout, stderr, code) = fx.run("sh -c 'cat <&3; echo inner_rc=$?' 3</dev/null\n");
    assert_eq!(stdout, "inner_rc=0\n");
    assert!(!stderr.contains("Bad file descriptor"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Output kind: the child must be able to write through the slot.
#[cfg(unix)]
#[test]
fn issue2304_devnull_output_foreign_child() {
    let fx = fixture("output");
    let (stdout, stderr, code) = fx.run("sh -c 'echo x >&3; echo inner_rc=$?' 3>/dev/null\n");
    assert_eq!(stdout, "inner_rc=0\n");
    assert!(!stderr.contains("Bad file descriptor"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Read-write kind: ONE description serving both directions — write
/// succeeds, the read then sees EOF (the null device, not a file).
#[cfg(unix)]
#[test]
fn issue2304_devnull_readwrite_foreign_child() {
    let fx = fixture("readwrite");
    let (stdout, stderr, code) =
        fx.run("sh -c 'echo x >&3; cat <&3; echo inner_rc=$?' 3<>/dev/null\n");
    assert_eq!(stdout, "inner_rc=0\n");
    assert!(!stderr.contains("Bad file descriptor"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Nested-rubash observer: the delivered slot must survive the
/// child's own startup replay (the env channel + init restore), not
/// just a foreign child's raw inheritance.
#[cfg(unix)]
#[test]
fn issue2304_devnull_input_nested_rubash_child() {
    let fx = fixture("nested");
    let (stdout, stderr, code) =
        fx.run("\"$RUBASH_UNDER_TEST\" -c 'cat <&3; echo inner_rc=$?' 3</dev/null\n");
    assert_eq!(stdout, "inner_rc=0\n");
    assert!(!stderr.contains("Bad file descriptor"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Control: the stdio /dev/null redirects (fd 0/1, no number) are
/// heavily used and must not move.
#[cfg(unix)]
#[test]
fn issue2304_stdio_devnull_control() {
    let fx = fixture("stdio");
    let (stdout, _stderr, code) =
        fx.run("echo hi >/dev/null; echo out_rc=$?; cat </dev/null; echo in_rc=$?\n");
    assert_eq!(stdout, "out_rc=0\nin_rc=0\n");
    assert_eq!(code, Some(0));
}
