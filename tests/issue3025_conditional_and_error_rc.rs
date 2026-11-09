//! rubash#3025 — `[[ -t a && -z b ]]` must return 2, not 1.
//!
//! GNU evaluates a `[[ ... ]]` conditional node-by-node
//! (execute_cmd.c:3993-4001 `execute_cond_node`, COND_AND arm):
//! `result = execute_cond_node (cond->left); if (result ==
//! EXECUTION_SUCCESS) result = execute_cond_node (cond->right);`
//! A non-zero left result — including the error code 2 a `-t`
//! operand raises via test.c:106 `TEST_ERREXIT_STATUS` /
//! test.c:885-892 `cond_test` — propagates verbatim; the right side
//! is never evaluated. Rubash's `&&` arm in
//! `src/executor/conditional.rs` folded any non-zero left result to
//! the literal `1`, discarding the error code.
//!
//! Every expected value below was read off GNU bash 5.3.20
//! (`~/workspace/rubash-audit/build-gnu/bash-5.3/bash`).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level
//! stdout/stderr assertions. `LC_ALL=C.UTF-8` is set on the child
//! explicitly so the pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> (Vec<u8>, Vec<u8>, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        output.stdout,
        output.stderr,
        output.status.code().unwrap_or(-1),
    )
}

/// The issue's repro: error on the `&&` left side propagates rc 2.
#[test]
fn and_error_left_propagates_rc_2() {
    let (stdout, stderr, code) = run("[[ -t a && -z b ]]; echo $?");
    assert_eq!(stdout, b"2\n");
    assert_eq!(code, 0);
    assert!(
        String::from_utf8_lossy(&stderr).contains("[[: a: integer expected"),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// Control: lone `-t` with a non-integer operand errors with rc 2.
#[test]
fn lone_t_error_is_rc_2() {
    let (stdout, stderr, code) = run("[[ -t a ]]; echo $?");
    assert_eq!(stdout, b"2\n");
    assert_eq!(code, 0);
    assert!(
        String::from_utf8_lossy(&stderr).contains("[[: a: integer expected"),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// Control: error on the `&&` right side is never reached — `-z b`
/// is false (the literal `b` is non-empty), so `&&` short-circuits
/// to 1 with no diagnostic at all.
#[test]
fn and_error_right_short_circuits_to_rc_1() {
    let (stdout, stderr, code) = run("[[ -z b && -t a ]]; echo $?");
    assert_eq!(stdout, b"1\n");
    assert_eq!(code, 0);
    assert!(
        stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// Control: `||` with an erroring left side evaluates the right side
/// and returns its result (1 here).
#[test]
fn or_error_left_evaluates_right_rc_1() {
    let (stdout, stderr, code) = run("[[ -t a || -z b ]]; echo $?");
    assert_eq!(stdout, b"1\n");
    assert_eq!(code, 0);
    assert!(
        String::from_utf8_lossy(&stderr).contains("[[: a: integer expected"),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// Both sides error: only the left diagnostic fires (right never
/// evaluated) and its rc 2 propagates.
#[test]
fn and_both_error_only_left_diagnostic_rc_2() {
    let (stdout, stderr, code) = run("[[ -t a && -t b ]]; echo $?");
    assert_eq!(stdout, b"2\n");
    assert_eq!(code, 0);
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("[[: a: integer expected"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("[[: b:"),
        "right side must not be evaluated; stderr: {stderr}"
    );
}

/// `!` inverts by the GNU rule (execute_cmd.c:4125-4126): any
/// non-success result — including error 2 — becomes success.
#[test]
fn bang_inverts_error_rc_2_to_0() {
    let (stdout, stderr, code) = run("[[ ! -t a ]]; echo $?");
    assert_eq!(stdout, b"0\n");
    assert_eq!(code, 0);
    assert!(
        String::from_utf8_lossy(&stderr).contains("[[: a: integer expected"),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// `||` after a failed `&&`: the erroring `&&` yields 2 (not
/// success), so the `||` right side runs and returns 0.
#[test]
fn or_after_error_and_evaluates_right() {
    let (stdout, stderr, code) = run("[[ -t a && -z b || -n c ]]; echo $?");
    assert_eq!(stdout, b"0\n");
    assert_eq!(code, 0);
    assert!(
        String::from_utf8_lossy(&stderr).contains("[[: a: integer expected"),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}
