//! rubash#3502 — `${v:?${-x}}`: GNU's parameter_brace_expand_error
//! expands the `:?` message word FIRST (subst.c:8224 expand_string); a
//! bad substitution inside that word reports its own diagnostic and
//! longjmps before the outer `name: word` report can run, so the output
//! is only `${-x}: bad substitution` — never a trailing `v: ` line.
//!
//! Expectations verified against WSL GNU Bash 5.3.20 script-file probes
//! (scratch-h3/h3-3502-probe.sh -> out/3502-oracle.txt).

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
fn question_mark_bad_substitution_word_reports_inner_only() {
    let (stdout, stderr, code) = rubash("unset v; echo \"${v:?${-x}}\"\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("v: "),
        "outer :? report must be suppressed, stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn question_no_colon_bad_substitution_word_reports_inner_only() {
    let (stdout, stderr, code) = rubash("unset v; echo \"${v?${-x}}\"\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("v: "), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn question_mark_null_bad_substitution_word_reports_inner_only() {
    let (stdout, stderr, code) = rubash("v=; echo \"${v:?${-x}}\"\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("v: "), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn question_mark_bad_substitution_names_the_whole_message_word() {
    // GNU expand_string judges the whole `:?` word part: the diagnostic
    // subject is `pre ${-x} post`, not the inner `${-x}` fragment.
    let (stdout, stderr, code) = rubash("unset v; echo \"${v:?pre ${-x} post}\"\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("pre ${-x} post: bad substitution"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("v: "), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn question_mark_nested_question_error_still_names_inner_variable() {
    // rubash#2343 behavior preserved: a nested `:?` inside the message
    // word reports the INNER variable and stays fatal (rc 127 in GNU).
    let (stdout, stderr, code) = rubash("unset v x; echo \"${v:?${x:?inner}}\"\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("x: inner"), "stderr: {stderr}");
    assert!(!stderr.contains("v: "), "stderr: {stderr}");
    assert_eq!(code, Some(127));
}

#[test]
fn question_mark_message_with_flags_parameter_unchanged() {
    // `${-}` is a valid special parameter: the message expands to the
    // shell flags and `v: <flags>` is still reported (GNU rc 127).
    let (stdout, stderr, code) = rubash("unset v; echo \"${v:?${-}}\"\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("v: "), "stderr: {stderr}");
    assert_eq!(code, Some(127));
}

#[test]
fn bad_substitution_before_question_error_wins() {
    // GNU expands left-to-right: `${-y}` errors before `${a:?x}` is
    // reached, so the bad substitution — not `a: x` — is reported.
    let (stdout, stderr, _) = rubash("unset a; echo \"${-y}${a:?x}\"\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("bad substitution"), "stderr: {stderr}");
    assert!(!stderr.contains("a: x"), "stderr: {stderr}");
}
