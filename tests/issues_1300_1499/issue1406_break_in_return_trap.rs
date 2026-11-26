//! Issue rubash#1406: `break`/`continue` in function's RETURN trap leaks
//! into the caller's loop (GNU errors).
//!
//! GNU runs the RETURN trap in the function's return context, which has no
//! loop, so `break` errors with "only meaningful in a loop" and the caller's
//! loop continues. Rubash leaked the Break into the caller's loop.
//!
//! Fix: src/executor/trap_exec.rs run_return_trap clears loop_depth around
//! the trap action.
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
fn break_in_return_trap_does_not_leak() {
    let (stdout, stderr, code) = rubash(
        r#"f() { trap 'echo R; break' RETURN; true; }; for i in 1 2 3; do f; echo "iter=$i"; done; echo done"#,
    );
    assert_eq!(code, Some(0));
    // All 3 iterations run; break errors but doesn't kill the loop.
    assert_eq!(stdout.matches("R\n").count(), 3);
    assert_eq!(stdout.matches("iter=").count(), 3);
    assert!(stdout.contains("done"));
    assert!(stderr.contains("only meaningful in a"));
}
