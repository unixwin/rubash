//! Issue unixwin/rubash#4047 — redirect failures on simple commands inside
//! async subshell / brace-group bodies must not be swallowed.
//!
//! GNU `print_cmd.c:1013` (`print_simple_command`) prints every redirection in
//! `simple_command->redirects` via `print_redirection_list`. Rubash's
//! `bash_command_text` only serialized the stdio mirror fields (`0..=2`),
//! dropping list-only redirects (`fd >= 3` and `{var}` dynamic fds) when
//! serializing simple commands inside respawned async `&` bodies.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn async_brace_group_simple_command_failing_move_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("{ echo hi 8>&10-; } &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_brace_group_simple_command_failing_dup_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("{ true >&3; } &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_subshell_simple_command_failing_move_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("( echo hi 8>&10- ) &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_simple_command_failing_move_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("echo hi 8>&10- &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_brace_group_simple_command_failing_var_fd_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("{ echo hi {v}>&10-; } &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_brace_group_outer_failing_move_redirect() {
    let (stdout, stderr, code) = run_rubash_stdin("{ echo hi; } 8>&10- &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc=1", "stdout={stdout:?}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor diagnostic on stderr, got: {stderr:?}"
    );
}

#[test]
fn async_brace_group_simple_command_valid_fd_dup_succeeds() {
    let (stdout, stderr, code) = run_rubash_stdin("{ echo hi 8>&1; } &\nwait $!\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.lines().collect::<Vec<_>>(), vec!["hi", "rc=0"]);
    assert!(stderr.is_empty(), "unexpected stderr: {stderr:?}");
}
