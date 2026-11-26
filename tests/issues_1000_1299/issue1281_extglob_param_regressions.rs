//! rubash#1281 — a same-line `shopt -s extglob` did not take effect for a
//! following `${var%%+(pat)}` expansion: the whole script died with a
//! parse error (rc=2) instead of expanding with extglob enabled.
//!
//! GNU: `shopt` is a builtin executed in order; expansions later on the
//! same line see the new setting. Probe: clusters/k5/repro/r1281.sh.
//! Expectations byte-verified against GNU bash 5.3.20 (script files).

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
fn extglob_enabled_same_line_applies_to_param_expansion() {
    let (stdout, stderr, code) = rubash("shopt -s extglob; v=abc; echo C=${v%%+(b)*}\necho AFTER");
    assert_eq!(stdout, "C=a\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn extglob_param_expansion_control_without_same_line_shopt() {
    // Control: extglob enabled on an earlier line behaves the same.
    let (stdout, stderr, code) = rubash("shopt -s extglob\nv=abc; echo C=${v%%+(b)*}");
    assert_eq!(stdout, "C=a\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
