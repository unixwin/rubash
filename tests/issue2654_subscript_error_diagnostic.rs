//! rubash#2654 — an arithmetic error inside an array subscript was
//! silently swallowed: `((array[$index]++))` with
//! `index='0],b[1'` exited 1 with no diagnostic, where GNU reports
//! the subscript's own evalexp failure.
//!
//! GNU anchor: subst.c:11107 expand_array_subscript (reached from
//! expand_word_internal's `[` arm, subst.c:11395-11405, under
//! Q_ARITH): the subscript text is expanded as a unit and backslash-
//! quoted (abstab = `[`, `]`, `$`, `` ` ``, `~`; subst.c:11120-11122)
//! before being spliced back, so a `]`/`[` produced by an expansion
//! cannot open or close the subscript. evalexp then evaluates the
//! subscript through array_expand_index (arrayfunc.c:1368-1378) as
//! its own frame, and its evalerror (expr.c:1524-1535) names the
//! subscript text: `0],b[1: arithmetic syntax error: invalid
//! arithmetic operator (error token is "],b[1")`.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`. `LC_ALL=C.UTF-8`
//! is set on the child explicitly so the pin is hermetic.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

const NEEDLE: &str =
    "0],b[1: arithmetic syntax error: invalid arithmetic operator (error token is \"],b[1\")";

/// The command fails rc 1 AND reports GNU's subscript diagnostic.
fn expect_subscript_error(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout, expected_stdout,
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(NEEDLE),
        "{ctx}: stderr must contain GNU's subscript diagnostic {NEEDLE:?}; got {stderr}"
    );
}

#[test]
fn arith_command_subscript_error_is_reported() {
    expect_subscript_error(
        "declare -a array; index='0],b[1'; ((array[$index]++)); echo rc=$?",
        b"rc=1\n",
        "((array[$index]++))",
    );
}

#[test]
fn arith_expansion_subscript_error_is_reported() {
    // The expansion error discards the echo command entirely.
    expect_subscript_error(
        "declare -a array; index='0],b[1'; echo $((array[$index]++)); echo rc=$?",
        b"rc=1\n",
        "$((array[$index]++))",
    );
}

#[test]
fn assignment_subscript_error_is_reported() {
    expect_subscript_error(
        "declare -a array; index='0],b[1'; array[$index]=5; echo rc=$? a0=${array[0]:-UNSET}",
        b"rc=1 a0=UNSET\n",
        "array[$index]=5",
    );
}

#[test]
fn braced_subscript_error_is_reported() {
    expect_subscript_error(
        "declare -a array; index='0],b[1'; echo ${array[$index]}; echo rc=$?",
        b"rc=1\n",
        "${array[$index]}",
    );
}

#[test]
fn literal_comma_expression_still_evaluates() {
    // Control: the same final text written literally is a valid comma
    // expression — only the EXPANSION-produced brackets are protected.
    let out = run("declare -a array b; ((array[0],b[1]++)); echo rc=$? b1=${b[1]}");
    assert_eq!(out.stdout, b"rc=1 b1=1\n", "literal comma control");
    assert!(
        out.stderr.is_empty(),
        "literal comma control: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn ordinary_subscript_still_evaluates() {
    let out = run("declare -a array; i=1; ((array[$i]++)); echo ${array[1]}");
    assert_eq!(out.stdout, b"1\n", "ordinary subscript control");
    assert_eq!(out.code, 0, "ordinary subscript control rc");
    assert!(
        out.stderr.is_empty(),
        "ordinary subscript control: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
