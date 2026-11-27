//! rubash#1113 — Chained compound assignment `$((j+=j-=2))` evaluates
//! to 6 instead of 8.
//!
//! GNU expr.c: compound assignment is right-associative — the inner
//! `j -= 2` runs first (j: 5 -> 3, value 3), then the outer `j += 3`
//! stores and yields 8. Both the expansion value and the stored value
//! must be 8.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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
fn chained_compound_assignment_is_right_associative() {
    let (stdout, _, code) = rubash("j=5; echo $((j+=j-=2)) $j");
    assert_eq!(stdout, "8 8\n");
    assert_eq!(code, Some(0));
}
