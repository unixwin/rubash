//! Issue rubash#1405: `break`/`continue` in ERR-trap action silently
//! ignored inside loops (GNU honors it).
//!
//! GNU propagates Break/Continue unwinds from the ERR trap action to the
//! enclosing loop. Rubash swallowed them.
//!
//! Fix: src/executor/trap_exec.rs maybe_run_error_trap_inner now propagates
//! Break/Continue errors, preserving the failing command's status via
//! break_failure_status.
//!
//! Expected outputs verified against GNU bash.

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
fn break_in_err_trap_exits_loop() {
    let (stdout, _, code) = rubash(
        r#"for i in 1 2 3; do trap 'echo T; break' ERR; false; echo "iter=$i"; done; echo "done rc=$?""#,
    );
    assert_eq!(code, Some(0));
    // Trap's break exits loop on first failure: only one T, no iter lines.
    assert_eq!(stdout.matches("T\n").count(), 1);
    assert!(!stdout.contains("iter="));
    assert!(stdout.contains("done rc=1"));
}

#[test]
fn continue_in_err_trap_skips_iteration() {
    let (stdout, _, code) = rubash(
        r#"for i in 1 2 3; do trap 'echo T; continue' ERR; false; echo "iter=$i"; done; echo "done rc=$?""#,
    );
    assert_eq!(code, Some(0));
    // Each iteration's false triggers trap, continue skips the echo.
    assert_eq!(stdout.matches("T\n").count(), 3);
    assert!(!stdout.contains("iter="));
}
