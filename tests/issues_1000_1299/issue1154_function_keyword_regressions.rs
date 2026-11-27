//! rubash#1154 — function keyword degenerate cases beyond
//! bare-function (#1142 follow-up).
//!
//! GNU parse.y: `function f` must be followed by a compound command;
//! a bare `function f;`, a bare `function f` at end of input, and
//! `function f; echo ...` are all syntax errors with rc=2 and no
//! execution at all.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings):
//! message cores are GNU's; stdout stays empty in every form.

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
fn function_keyword_degenerate_forms_are_syntax_errors() {
    for (script, core) in [
        (
            "function f;\necho AFTER",
            "syntax error near unexpected token `;'",
        ),
        ("function f", "syntax error: unexpected end of file"),
        (
            "function f; echo AFTER",
            "syntax error near unexpected token `;'",
        ),
    ] {
        let (stdout, stderr, code) = rubash(script);
        assert_eq!(stdout, "", "script {script:?}");
        assert!(stderr.contains(core), "script {script:?} stderr: {stderr}");
        assert_eq!(code, Some(2), "script {script:?}");
    }
}
