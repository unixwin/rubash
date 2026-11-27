//! niubash#275: `read` on a record that is only the delimiter must succeed.
//!
//! GNU builtins/read.def reads the delimiter (read.def:731-738), assigns the
//! empty record and returns EXECUTION_SUCCESS; only a read that exhausts the
//! input before any delimiter (read.def:762-768, :949) fails the builtin.
//! The inherited-stdin and coproc transports used to return None for the
//! delimiter-only record — `printf '\n' | read v` exited 1 and a `while
//! read` loop silently skipped empty lines.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `printf '\n' | read v` — empty line is a served record, rc 0 (WSL
/// 5.2.21 oracle: rc=0).
#[test]
fn empty_line_read_succeeds() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "read v; echo \"rc=$?\""], b"\n");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Empty lines inside a `while read` loop are iterations, not loop exits:
/// `printf 'a\n\nb\n'` yields three records (GNU prints a, empty, b).
#[test]
fn while_read_keeps_empty_line_iteration() {
    let (stdout, stderr, rc) = run_stdin(
        &[
            "-c",
            "while read v; do echo \"got[$v]\"; done; echo \"rc=$?\"",
        ],
        b"a\n\nb\n",
    );
    assert_eq!(
        stdout, "got[a]\ngot[]\ngot[b]\nrc=0\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

/// A delimiter-only record still assigns the (empty) variable.
#[test]
fn empty_line_assigns_empty_value() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "read v; echo \"rc=$? len=${#v}\""], b"\n");
    assert_eq!(stdout, "rc=0 len=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Guard: true EOF (no bytes at all) still fails the builtin — the empty
/// record and EOF must stay distinguishable (GNU `read </dev/null` rc=1).
#[test]
fn real_eof_still_fails() {
    let (_stdout, stderr, rc) = run_stdin(&["-c", "read v; echo \"rc=$?\""], b"");
    assert_eq!(_stdout, "rc=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Guard: a partial record at EOF without the delimiter still assigns but
/// fails (read.def:949 eof rule, GNU `printf 'x' | read v` rc=1, v=x).
#[test]
fn partial_record_at_eof_fails_with_assignment() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "read v; echo \"rc=$? v=[$v]\""], b"x");
    assert_eq!(stdout, "rc=1 v=[x]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}
