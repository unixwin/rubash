//! rubash#1408 — `bg <PID>` accepted a bare PID operand and resumed
//! the job; GNU rejects it: a bare number that is not a job id warns
//! "job specification requires leading `%'" and fails "no such job"
//! (rc=1, job NOT resumed). A bare number that IS a job id resolves
//! with the same warning and reports "already in background" (rc=0).
//!
//! Probes: clusters/k9/probes/p1408{,b,c}.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).
//! (Cleanup uses `kill -9`: a plain SIGTERM stays pending on the
//! STOPPED job and the orphan would hold the captured stdout pipe
//! open — a harness artifact, not part of the pinned behavior.)

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn bg_bare_pid_is_rejected() {
    let (stdout, _stderr, code) = rubash(
        "set -m\nsleep 2 & p=$!\nbg $p 2>&1; echo \"rc=$?\"\njobs\nkill -9 $p 2>/dev/null; wait 2>/dev/null; true",
    );
    assert!(
        stdout.contains("job specification requires leading `%'"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("no such job"), "stdout: {stdout}");
    assert!(stdout.contains("rc=1"), "stdout: {stdout}");
    assert!(stdout.contains("Running"), "jobs output: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn bg_bare_pid_does_not_resume_stopped_job() {
    let (stdout, _stderr, code) = rubash(
        "set -m\nsleep 2 & p=$!\nkill -STOP $p; sleep 0.3\nbg $p 2>&1; echo \"rc=$?\"\njobs\nkill -9 $p 2>/dev/null; wait 2>/dev/null; true",
    );
    assert!(stdout.contains("no such job"), "stdout: {stdout}");
    assert!(stdout.contains("rc=1"), "stdout: {stdout}");
    assert!(
        stdout.contains("Stopped"),
        "job must stay stopped: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn bg_bare_job_number_resolves_with_warning() {
    let (stdout, _stderr, code) = rubash(
        "set -m\nsleep 2 & p=$!\nbg 1 2>&1; echo \"rc-num1=$?\"\nbg sleep 2>&1; echo \"rc-name=$?\"\nkill -9 $p 2>/dev/null; wait 2>/dev/null; true",
    );
    assert_eq!(
        stdout
            .matches("job specification requires leading `%'")
            .count(),
        2,
        "stdout: {stdout}"
    );
    assert_eq!(
        stdout.matches("already in background").count(),
        2,
        "stdout: {stdout}"
    );
    assert!(stdout.contains("rc-num1=0"), "stdout: {stdout}");
    assert!(stdout.contains("rc-name=0"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
