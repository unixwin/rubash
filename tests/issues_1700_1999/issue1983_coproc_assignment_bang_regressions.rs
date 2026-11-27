//! rubash#1983 — a `!` directly after an assignment first word of
//! the coproc's simple command is NOT a syntax error.
//!
//! GNU's lexer returns an assignment-shaped first word as
//! ASSIGNMENT_WORD, and parse.y:5899 reserved_word_acceptable has
//! no ASSIGNMENT_WORD in its accept set, so the following `!`
//! never becomes BANG: it lexes as a plain WORD and lands as the
//! child command's NAME (`coproc X=1 ! echo hi` runs a child that
//! reports `!: command not found`, script rc 0). The #1693
//! implementation of #1652 rejected every index-1 `!`, assignment
//! first word or not — rc 2. The glued shape `coproc X=1! echo hi`
//! (one word token, the `!` never starts a token) was never
//! affected and stays pinned as a control.
//!
//! Negative controls pin the exemption's edge: a first word that
//! merely CONTAINS `=` is not an assignment word — `9a=1` (digit
//! start) and `a-b=1` (hyphen) stay WORDs in GNU, so the `!` after
//! them is still BANG and still a syntax error.
//!
//! Expectations verified against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1983_coproc_bang_after_assignment_word_is_child_command_name() {
    // Parse-admission face only: no syntax error, the `!`-named
    // child runs (and fails its lookup), the script completes.
    // The coproc's wait status is deliberately NOT pinned here:
    // after the output fd drains, GNU forgets the status (wait
    // reports 0 deterministically) while rubash's async reap
    // races it (0 or the child's 127) — a pre-existing reap-face
    // divergence orthogonal to #1983's admission question.
    for script in [
        "coproc X=1 ! echo hi; echo parse_ok",
        "coproc a+=1 ! echo hi; echo parse_ok",
        "coproc _a9= ! echo hi; echo parse_ok",
    ] {
        let (stdout, stderr, code) = rubash_c(script);
        assert!(
            !stderr.contains("syntax error"),
            "script: {script}, stderr: {stderr}"
        );
        assert!(
            stderr.contains("!: command not found"),
            "script: {script}, stderr: {stderr}"
        );
        assert_eq!(stdout, "parse_ok\n", "script: {script}");
        assert_eq!(code, Some(0), "script: {script}");
    }
}

#[test]
fn issue1983_coproc_bang_after_subscript_assignment_word_parses() {
    // GNU accepts the shape (the child's own `a[1]` identifier
    // diagnostic is an executor face, not the parse admission this
    // issue owns): no syntax error, the `!`-named child runs and
    // fails as a command lookup, the script completes rc 0.
    let script = "coproc a[1]=x ! echo hi; echo parse_ok";
    let (stdout, stderr, code) = rubash_c(script);
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert!(stderr.contains("!: command not found"), "stderr: {stderr}");
    assert_eq!(stdout, "parse_ok\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1983_coproc_bang_after_non_assignment_word_still_syntax_error() {
    for script in [
        "coproc 9a=1 ! echo hi; echo rc=$?",
        "coproc a-b=1 ! echo hi; echo rc=$?",
    ] {
        let (stdout, stderr, code) = rubash_c(script);
        assert_eq!(stdout, "", "script: {script}");
        assert!(
            stderr.contains("syntax error near unexpected token `!'"),
            "script: {script}, stderr: {stderr}"
        );
        assert_eq!(code, Some(2), "script: {script}");
    }
}

#[test]
fn issue1983_coproc_glued_assignment_bang_still_runs() {
    // Control: the glued form is a single word token (value `1!`
    // for X) — it parsed before #1983 and must keep parsing.
    let script = "coproc X=1! echo hi; cat <&${COPROC[0]}; wait $COPROC_PID; echo rc=$?";
    let (stdout, stderr, code) = rubash_c(script);
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
