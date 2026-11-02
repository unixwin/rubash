//! Issue unixwin/rubash#1219: `exit N` inside a `>(...)` write-side
//! process substitution must terminate only the substitution child, not
//! the shell.
//!
//! GNU owner: subst.c:6362 process_substitute() forks the `>(...)` body;
//! the child ends through `exit(rc)` (subst.c:6628-6629) after
//! restore_original_signals cancelled the parent's traps (subst.c:6421).
//! Verified against WSL GNU Bash 5.3.20: `echo hi > >(exit 3)` leaves the
//! parent at rc=0 and the script continues; a parent EXIT trap fires once
//! at real shell exit, not inside the substitution body.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn exit_in_write_procsub_is_child_local() {
    // GNU 5.3.20: prints rc=0 + done, shell exit status 0.
    let (stdout, stderr, code) = rubash_stdin("echo hi > >(exit 3)\necho \"rc=$?\"\necho done\n");
    assert_eq!(stdout, "rc=0\ndone\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn exit_in_write_procsub_after_body_output() {
    // Delivery is complete but ORDER between the consumer's output and
    // the parent's next line is a race in GNU itself (captain
    // adjudication on rubash#2525, W3 evidence): over 20 runs of this
    // harness GNU produced `after\ninner` 15 times, `inner\nafter`
    // twice, and lost the parent line entirely 3 times; rubash
    // deterministically produces GNU's modal order. The pin therefore
    // asserts complete delivery + continuation + status, not one
    // particular interleaving.
    let (stdout, stderr, code) = rubash_stdin("echo hi > >(echo inner; exit 3)\necho after\n");
    assert!(
        stdout == "inner\nafter\n" || stdout == "after\ninner\n",
        "expected both lines delivered, got {stdout:?}; stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn bare_exit_in_write_procsub_is_child_local() {
    // Bare `exit` (no argument) still ends only the child; GNU continues.
    let (stdout, stderr, code) = rubash_stdin("echo hi > >(exit)\necho ok\n");
    assert_eq!(stdout, "ok\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn exit_in_write_procsub_does_not_fire_parent_exit_trap() {
    // GNU: restore_original_signals cancels the parent's EXIT trap in the
    // forked child — TRAP prints once (at real shell exit), not twice.
    let (stdout, stderr, code) =
        rubash_stdin("trap 'echo TRAP' EXIT\necho hi > >(exit 3)\necho done\n");
    assert_eq!(stdout, "done\nTRAP\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
