//! Issue rubash#1523: an output process substitution in an
//! assignment's right-hand side (`v=>(...)`) was not finished at
//! its own command. The queued body drained at the NEXT command's
//! finish (diagnostics attributed to that command's line), a
//! failing body ran twice and its error propagated into the
//! draining command (polluting `$?` / killing the script), a
//! trailing assignment's body never ran at all, and the body's
//! diagnostics were attributed to the ambient line because the
//! assignment finish path never seeded the construction line.
//!
//! GNU model: `>(...)` forks at materialization (subst.c
//! process_substitute); the body is a child process whose status
//! is unobservable to the parent script — it runs exactly once,
//! its diagnostics carry the line of the command that constructed
//! it, and the constructing command's own status is unaffected.
//! Rubash's in-process model must reproduce that contract at the
//! assignment command's own completion: run once, attribute to
//! the construction line, never leak the body's status.
//!
//! The sibling word form (`echo x > >(tr ...)`) already satisfied
//! the contract (rubash#1184 f4 line seeding); it is pinned here
//! as the control. Expected outputs verified against GNU bash
//! 5.3.20 (script files).

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
fn issue1523_standalone_assignment_procsub_runs_once_at_own_line() {
    // Issue case: standalone assignment at line 4. GNU: the
    // diagnostic fires once, at line 4; the script's status is
    // the last command's (0).
    let (stdout, stderr, code) =
        rubash("echo a\necho b\necho c\nv=>(tr a-z A-Z >&3)\nsleep 0.3\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\nb\nc\nrc=0\n");
    assert_eq!(
        stderr.matches("Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("line 4:"), "stderr: {stderr}");
}

#[test]
fn issue1523_temp_assignment_procsub_runs_once() {
    // The temporary-assignment form (`v=>(...) true`): GNU runs
    // the body once, attributed to the assignment's own line 2.
    let (stdout, stderr, code) = rubash("echo a\nv=>(tr a-z A-Z >&3) true\nsleep 0.3\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\nrc=0\n");
    assert_eq!(
        stderr.matches("Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("line 2:"), "stderr: {stderr}");
}

#[test]
fn issue1523_two_assignments_each_body_runs_once() {
    // Two assignment procsubs on one line: each body runs exactly
    // once (fds 3 and 4), both attributed to line 2.
    let (_, stderr, code) =
        rubash("echo a\na=>(tr a-z A-Z >&3) b=>(tr a-z A-Z >&4)\nsleep 0.3\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("3: Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("4: Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(stderr.matches("line 2:").count(), 2, "stderr: {stderr}");
}

#[test]
fn issue1523_trailing_assignment_body_still_runs() {
    // The assignment is the script's last command: the body must
    // still run exactly once (previously it never ran).
    let (stdout, stderr, code) = rubash("echo a\nv=>(tr a-z A-Z >&3)");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\n");
    assert_eq!(
        stderr.matches("Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("line 2:"), "stderr: {stderr}");
}

#[test]
fn issue1523_failing_body_does_not_leak_status() {
    // The body's exit status is the child process's business:
    // the script must continue and its own status stay clean
    // (previously the body ran twice and its error killed the
    // script with the body's status).
    let (stdout, _, code) = rubash("v=>(exit 3)\necho after\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\nrc=0\n");
}

#[test]
fn issue1523_word_form_control_unchanged() {
    // Control: the sibling simple-command word form already ran
    // its body once at the construction line; the assignment
    // finish rework must not disturb it.
    let (stdout, stderr, code) =
        rubash("echo a\necho b\necho c\necho x > >(tr a-z A-Z >&3)\nsleep 0.3\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a\nb\nc\nrc=0\n");
    assert_eq!(
        stderr.matches("Bad file descriptor").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("line 4:"), "stderr: {stderr}");
}
