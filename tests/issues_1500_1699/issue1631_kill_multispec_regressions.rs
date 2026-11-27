//! rubash#1631 — `kill` with more than one signal specification: GNU
//! builtins/kill.def's option loop consumes only the FIRST spec (the
//! bare -SIG branch is gated on `saw_signal == 0`); a later -SIG word
//! ends the scan and is judged as a pid OPERAND — `` `-TERM': not a
//! pid or valid job spec `` for a name, a process-group delivery
//! attempt for a negative number (`(-15) - No such process`). The -s/-n
//! forms stay live after a bare spec. The tracked-job pre-parser in the
//! executor used to let every later -SIG silently override the first,
//! so the victim died by the WRONG signal with no diagnostic at all.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle, script
//! files): victim = the shell's own `/usr/bin/sleep 30` background job,
//! so the executor's tracked-job path is the one under test.

#![cfg(unix)]

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
fn issue1631_second_bare_spec_is_badpid_operand_first_spec_delivered() {
    let (stdout, stderr, code) = rubash(
        "/usr/bin/sleep 30 & v=$!\n\
         kill -s USR1 -TERM $v; echo kill_rc=$?\n\
         wait $v; echo wait_rc=$?",
    );
    assert!(
        stderr.contains("`-TERM': not a pid or valid job spec"),
        "stderr: {stderr}"
    );
    // USR1 (10) was the delivered signal: 128 + 10 = 138, kill itself
    // succeeded (the diagnostic touches neither GNU counter).
    assert!(stdout.contains("kill_rc=0\n"), "stdout: {stdout}");
    assert!(stdout.contains("wait_rc=138\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1631_negative_number_after_spec_is_group_delivery_attempt() {
    let (stdout, stderr, code) = rubash(
        "/usr/bin/sleep 30 & v=$!\n\
         kill -n 10 -15 $v; echo kill_rc=$?\n\
         wait $v; echo wait_rc=$?",
    );
    assert!(
        stderr.contains("(-15) - No such process"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("kill_rc=0\n"), "stdout: {stdout}");
    assert!(stdout.contains("wait_rc=138\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1631_dash_s_after_bare_spec_still_selects_signal() {
    let (stdout, stderr, code) = rubash(
        "/usr/bin/sleep 30 & v=$!\n\
         kill -TERM -s USR1 $v; echo kill_rc=$?\n\
         wait $v; echo wait_rc=$?",
    );
    // GNU prints the job death notice on stderr when `wait` reaps
    // the USR1-killed victim — 5.3.20 oracle, both script forms,
    // 10/10 runs, stderr never empty (GATE-READJUDICATION A1; the
    // original pin's is_empty was a wrong pin). The notice's line
    // attribution wobbles between the kill and wait lines, so the
    // line number is NOT pinned; the notice shape is. What must stay
    // absent is any diagnostic from `kill` itself: `-s` after a bare
    // spec is a legal continuation, not a badpid operand.
    assert!(
        stderr.contains("User defined signal 1"),
        "job death notice missing; stderr: {stderr}"
    );
    assert!(
        stderr.contains("/usr/bin/sleep 30"),
        "death notice must name the job command; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("not a pid or valid job spec"),
        "kill emitted a badpid diagnostic; stderr: {stderr}"
    );
    assert!(stdout.contains("kill_rc=0\n"), "stdout: {stdout}");
    assert!(stdout.contains("wait_rc=138\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1631_untracked_negative_and_badpid_operands() {
    // No tracked job: the whole command falls to the builtin, whose
    // operand loop must produce BOTH diagnostics and rc=1.
    let (stdout, stderr, code) = rubash("kill -15 -TERM 999999; echo rc=$?");
    assert!(
        stderr.contains("`-TERM': not a pid or valid job spec"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("rc=1\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));

    // A negative operand after the spec is a group delivery attempt,
    // not a second spec and not a badpid.
    let (stdout, stderr, code) = rubash("kill -TERM -999999; echo rc=$?");
    assert!(
        stderr.contains("(-999999) - No such process"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("rc=1\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
