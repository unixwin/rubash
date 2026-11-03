//! Issue rubash#1403: exit/return silently wrap integers outside int64
//! range (GNU: numeric argument required, rc=2).
//!
//! GNU parses the status as int64; out-of-range values are rejected.
//! Rubash parsed as i128 and silently wrapped.
//!
//! Fix: src/builtins/exit.rs parse_exit_status and
//! src/executor/pwd_loop_builtins.rs return parsing now use i64.
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
fn exit_rejects_out_of_range_int() {
    let (_, stderr, code) = rubash("(exit 9223372036854775808)");
    // Subshell exit with out-of-range: numeric error, rc=2.
    assert!(stderr.contains("numeric argument required"));
    assert_eq!(code, Some(2));
}

#[test]
fn return_rejects_out_of_range_int() {
    let (_, stderr, code) = rubash("f() { return 9999999999999999999999; }; f");
    assert!(stderr.contains("numeric argument required"));
    assert_eq!(code, Some(2));
}
