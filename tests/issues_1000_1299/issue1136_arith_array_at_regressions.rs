//! rubash#1136 — `$((arr[@]))` error path is wrong: different
//! diagnostic, no value printed, exit 1.
//!
//! GNU expr.c / subst.c: `arr[@]` inside arithmetic names the whole
//! array as a subscript, which is a `bad array subscript` diagnostic —
//! but the expansion still evaluates (to 0) and the enclosing command
//! succeeds. It is not an arithmetic syntax error and must not abort
//! the command line.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string).

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
fn arith_array_at_is_bad_subscript_but_evaluates() {
    let (stdout, stderr, code) = rubash("arr=(3 4); echo \"val:$((arr[@]))\"; echo rc=$?");
    assert_eq!(stdout, "val:0\nrc=0\n");
    assert!(
        stderr.contains("arr[@]: bad array subscript"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
