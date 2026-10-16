//! Issue rubash#661: the `if`-condition fast path must not fire for a
//! shadowed `:`/`true`.
//!
//! The unmarked fast path skips the condition's errexit suppression. A
//! `true()` function (or alias) returning nonzero under `set -e` must
//! keep the slow path: GNU runs the ELSE branch and continues (rc 0).
//! Every expectation below is the literal GNU bash 5.3 output.

use std::process::Command;

fn run(source: &str) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn shadowing_function_keeps_slow_path_under_errexit() {
    let (out, code) = run("true() { return 1; }\nset -e\nif true; then echo THEN; else echo ELSE; fi\necho AFTER");
    assert_eq!(out, "ELSE\nAFTER\n");
    assert_eq!(code, 0);
}

#[test]
fn shadowing_alias_keeps_slow_path_under_errexit() {
    let (out, code) = run("shopt -s expand_aliases\nalias true=false\nset -e\nif true; then echo THEN; else echo ELSE; fi\necho AFTER");
    assert_eq!(out, "ELSE\nAFTER\n");
    assert_eq!(code, 0);
}

#[test]
fn plain_true_still_takes_fast_path() {
    let (out, code) = run("set -e\nif true; then echo THEN; else echo ELSE; fi\necho AFTER");
    assert_eq!(out, "THEN\nAFTER\n");
    assert_eq!(code, 0);
}

#[test]
fn plain_colon_condition_unchanged() {
    let (out, code) = run("if :; then echo THEN; else echo ELSE; fi");
    assert_eq!(out, "THEN\n");
    assert_eq!(code, 0);
}
