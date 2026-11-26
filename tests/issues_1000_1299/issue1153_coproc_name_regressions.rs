//! rubash#1153 — coproc NAME simple-command silently dropped;
//! coproc "" rc/message diverge.
//!
//! GNU execute_cmd.c (coproc handling): `coproc NAME { compound; }`
//! registers the coprocess under NAME (an fd array). With a SIMPLE
//! command, the first word is the command itself — `coproc CP echo hi`
//! runs a command named CP (an anonymous coproc), it does not name
//! the coproc CP. An empty name is `not a valid identifier`, rc=1.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings); the
//! fd numbers inside the coproc array are kernel-assigned, so the
//! named-coproc pin asserts the array's shape, not its values.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn named_coproc_compound_registers_fd_array() {
    let (stdout, _, code) = rubash("coproc CP { echo hi; }\ndeclare -p CP");
    assert!(
        stdout.starts_with("declare -a CP=([0]=\"") && stdout.contains("[1]=\""),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_word_before_simple_command_is_the_command() {
    let (stdout, stderr, code) =
        rubash("coproc CP echo hi\nsleep 0.2\ndeclare -p COPROC\necho \"pid=$COPROC_PID\"");
    assert_eq!(stdout, "pid=\n");
    assert!(stderr.contains("CP: command not found"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_empty_name_is_invalid_identifier() {
    let (stdout, stderr, code) = rubash("coproc \"\" { echo hi;}; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
