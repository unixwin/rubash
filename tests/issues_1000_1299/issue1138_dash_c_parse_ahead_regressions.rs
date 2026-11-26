//! rubash#1138 — A syntax error later in a `-c` string does not
//! prevent earlier commands from executing.
//!
//! GNU parse.y / execute_cmd.c: a `-c` string is parsed as a unit
//! before execution begins; a syntax error anywhere in it aborts the
//! whole string with rc=2 and NOTHING runs — including side effects
//! of the commands that precede the error.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): stdout
//! is empty, the exit status is 2, and the diagnostics name the
//! conditional syntax error.

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
fn dash_c_syntax_error_aborts_before_any_execution() {
    let (stdout, stderr, code) = rubash("echo hi; [[ a == ]]");
    assert_eq!(stdout, "", "no command may run before the parse error");
    assert!(
        stderr.contains("syntax error near `]]'") || stderr.contains("unexpected argument"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(2));
}
