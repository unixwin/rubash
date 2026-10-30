//! rubash#1652 — `coproc ! ...` must be a syntax error.
//!
//! GNU parse.y:1125-1175 coproc grammar is `COPROC [WORD]
//! shell_command`, and `!` is a pipeline modifier, not a
//! shell_command, so a `!` in the coproc's command position is
//! rejected with `syntax error near unexpected token `!'` and
//! status 2. Rubash collected the `!` as a simple-command word
//! and ran the coproc (the negation silently applying inside
//! the child).
//!
//! Two positions error, both verified against GNU bash 5.3.20:
//!  - body position: `coproc ! ...`, `coproc ! { ...; }`
//!    (the `!` keyword must not be stolen as a NAME either —
//!    that used to report "not a valid identifier", rc 1);
//!  - second word of the simple command: `coproc a ! ...`,
//!    `coproc NAME ! ...` (after COPROC WORD the parser commits
//!    to the named form and demands a shell_command).
//! A `!` after two or more words is a plain word and still
//! parses (`coproc a b ! c`, `coproc echo hi !`), as does a
//! quoted "!" (`coproc echo "!"`). A bare `coproc time ...`
//! keeps its existing behavior (see the #1652 notes: the
//! time-prefix coproc path itself diverges from GNU and is a
//! separate issue).
//!
//! Expectations verified against GNU bash. The error aborts the
//! script at the offending line: earlier lines run, later lines
//! do not, and the process exits 2.

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

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1652-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_bang_body_is_syntax_error() {
    let (stdout, stderr, code) = rubash_c("coproc ! echo hi; echo rc=$?");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("syntax error near unexpected token `!'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("`coproc ! echo hi; echo rc=$?'"), "stderr: {stderr}");
    assert_eq!(code, Some(2));
}

#[test]
fn coproc_bang_second_word_is_syntax_error() {
    for script in [
        "coproc a ! b; echo rc=$?",
        "coproc NAME ! echo hi; echo rc=$?",
        "coproc echo ! foo; echo rc=$?",
        "coproc a !; echo rc=$?",
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
fn coproc_bang_brace_name_is_syntax_error_not_identifier_error() {
    let (stdout, stderr, code) = rubash_c("coproc ! { echo hi; }; echo rc=$?");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("syntax error near unexpected token `!'"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("not a valid identifier"), "stderr: {stderr}");
    assert_eq!(code, Some(2));
}

#[test]
fn coproc_bang_aborts_script_at_offending_line() {
    let (stdout, stderr, code) =
        rubash_script("echo before\ncoproc a ! b\necho after\n");
    assert_eq!(stdout, "before\n");
    assert!(
        stderr.contains("syntax error near unexpected token `!'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("`coproc a ! b'"), "stderr: {stderr}");
    assert_eq!(code, Some(2));
}

#[test]
fn coproc_bang_after_two_words_still_parses() {
    for script in [
        "coproc a b ! c; echo rc=$?",
        "coproc NAME foo ! bar; echo rc=$?",
        "coproc echo hi !; echo rc=$?",
        "coproc echo \"!\"; cat <&${COPROC[0]}; wait $COPROC_PID; echo rc=$?",
    ] {
        let (stdout, stderr, code) = rubash_c(script);
        assert!(
            !stderr.contains("syntax error"),
            "script: {script}, stderr: {stderr}"
        );
        assert_eq!(code, Some(0), "script: {script}, stdout: {stdout}");
    }
}

#[test]
fn coproc_plain_still_works() {
    let (stdout, _, code) =
        rubash_c("coproc echo hi; cat <&${COPROC[0]}; wait $COPROC_PID; echo rc=$?");
    assert_eq!(stdout, "hi\nrc=0\n");
    assert_eq!(code, Some(0));
}
