//! rubash#1611 — `set -e` firing on a `time` command discarded the
//! timing report entirely (default and `-p` forms). GNU's
//! execute_time_command (execute_cmd.c) completes and prints its
//! report for any inner command that RETURNS to it — with a success,
//! failure, errexit, or exit-builtin status — and the errexit jump is
//! adjudicated by execute_time_command's CALLER afterwards; only a
//! `return` unwinding THROUGH the frame abandons the report (its
//! return_catch target lies outside). The bf-w3-a pin lives in
//! `issue1278_time_errexit_regressions.rs` and must pass once both
//! land.
//!
//! Expectations verified against GNU bash 5.3.20: reports are matched
//! on their stable heads (`real\t0m` default form, `real 0.` for -p)
//! because the elapsed digits vary per run.

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

fn assert_default_report(stderr: &str) {
    assert!(stderr.contains("real\t0m"), "stderr: {stderr}");
    assert!(stderr.contains("user\t0m"), "stderr: {stderr}");
    assert!(stderr.contains("sys\t0m"), "stderr: {stderr}");
}

#[test]
fn errexit_time_external_still_prints_default_report() {
    let (stdout, stderr, code) = rubash("set -e\ntime false\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_external_still_prints_posix_report() {
    let (stdout, stderr, code) = rubash("set -e\ntime -p false\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(stderr.contains("real 0."), "stderr: {stderr}");
    assert!(stderr.contains("user 0."), "stderr: {stderr}");
    assert!(stderr.contains("sys 0."), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_builtin_inner_still_prints() {
    let (stdout, stderr, code) = rubash("set -e\ntime test 1 = 2\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_subshell_inner_still_prints() {
    let (stdout, stderr, code) = rubash("set -e\ntime (exit 1)\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_brace_group_inner_still_prints() {
    let (stdout, stderr, code) = rubash("set -e\ntime { false; }\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_function_inner_still_prints() {
    let (stdout, stderr, code) = rubash("f() { return 3; }\nset -e\ntime f\necho lived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(3));
}

#[test]
fn errexit_time_err_trap_fires_and_report_prints() {
    let (stdout, stderr, code) = rubash("set -e\ntrap 'echo ERRFIRED' ERR\ntime false\necho lived");
    assert_eq!(stdout, "ERRFIRED\n", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn errexit_time_null_command_inner_fires_trap_and_prints() {
    // The timed command is a vanishing-words command whose substitution
    // imposed the failing status; GNU fires the ERR trap, prints the
    // report, and exits 1.
    let (stdout, stderr, code) =
        rubash("set -e\ntrap 'echo ERRFIRED' ERR\ntime $(false)\necho lived");
    assert_eq!(stdout, "ERRFIRED\n", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(1));
}

#[test]
fn time_exit_builtin_still_prints_report() {
    for script in ["time exit 3\necho never", "set -e\ntime exit 3\necho never"] {
        let (stdout, stderr, code) = rubash(script);
        assert_eq!(stdout, "", "script {script:?} stdout: {stdout}");
        assert_default_report(&stderr);
        assert_eq!(code, Some(3), "script {script:?}");
    }
}

#[test]
fn time_break_still_prints_report() {
    let (stdout, stderr, code) = rubash("while true; do time break; done\necho out");
    assert_eq!(stdout, "out\n", "stdout: {stdout}");
    assert_default_report(&stderr);
    assert_eq!(code, Some(0));
}

#[test]
fn time_return_crossing_drops_report() {
    // The one abandonment GNU keeps: `return` unwinds through the
    // timed frame to the function's return_catch — no report, the
    // function returns the status, execution continues after the call.
    let (stdout, stderr, code) =
        rubash("f() { time return 3; echo after-in-f; }\nf\necho done rc=$?");
    assert_eq!(stdout, "done rc=3\n", "stdout: {stdout}");
    assert!(!stderr.contains("real"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
