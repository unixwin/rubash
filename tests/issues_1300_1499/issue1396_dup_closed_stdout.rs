//! Issue rubash#1396: Dup of closed stdout (`>&- 2>&1`) runs the command
//! instead of failing the redirect.
//!
//! GNU fails at redirect setup time with "1: Bad file descriptor" rc=1
//! when `2>&1` dups from a closed fd 1. Rubash ran the command.
//!
//! Fix: src/executor/redirection.rs now marks CloseOutput/CloseInput in
//! the `opened` map, so later dups see the closed fd (GNU redir.c:798-803).
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
fn dup_closed_stdout_fails_at_redirect_time() {
    let (stdout, stderr, code) = rubash(r#"echo hi >&- 2>&1; echo "rc=$?""#);
    // Command does not run; redirect fails with EBADF.
    assert!(!stdout.contains("hi"));
    assert!(stderr.contains("Bad file descriptor"));
    assert!(stdout.contains("rc=1"));
    assert_eq!(code, Some(0));
}
