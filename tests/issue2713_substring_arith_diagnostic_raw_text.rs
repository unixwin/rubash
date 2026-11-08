//! rubash#2713 (deep-audit B1-10) — substring/arithmetic diagnostics must
//! echo the raw expression text exactly as written; rubash trims it.
//!
//! GNU anchors:
//! - Negative length: `verify_substring_values` (subst.c:8395) splits the
//!   length text out of the raw substring operand with `skiparith` and
//!   reports it untrimmed via `internal_error("%s: substring expression
//!   < 0", t)` (subst.c:8487, 8502). `${arr[@]: 1: -1}` therefore reports
//!   ` -1` (leading space preserved).
//! - Arithmetic: `evalerror` (expr.c:1526-1537) echoes the expression
//!   skipping only leading whitespace and reports the error token as the
//!   scanner's raw remainder (`lasttp`), trailing blanks included:
//!   `${v: '1' }` -> token `'1' ` and `${v: 1 + }` -> token `+ `.
//!
//! Expectations below are byte-measured against GNU bash 5.3.20
//! (probes in the E3 lane REPORT). Harness mirrors
//! issue3024_unset_substring_arith_shortcircuit: `rubash -c` via
//! `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8` pinned on the child.

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

/// Fatal expansion diagnostic: rc 1, empty stdout, stderr contains needle.
fn expect_diag(script: &str, needle: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code, 1,
        "{ctx}: rc must be 1; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{ctx}: stdout must be empty on a fatal expansion error; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
}

#[test]
fn array_negative_length_echoes_raw_text_with_leading_space() {
    // GNU: the length slice after the separator colon is " -1" verbatim,
    // so the line reads "...:  -1: substring expression < 0" (two spaces).
    expect_diag(
        "arr=(a b c); echo ${arr[@]: 1: -1}",
        "  -1: substring expression < 0",
        "A1 array negative length, raw text",
    );
}

#[test]
fn scalar_negative_length_echoes_raw_text_with_leading_space() {
    expect_diag(
        "v=abcdef; echo ${v:5: -2}",
        "  -2: substring expression < 0",
        "A2 scalar adjusted negative length, raw text",
    );
}

#[test]
fn array_negative_length_without_space_is_untrimmed_control() {
    // Control: no space in the source, so GNU echoes "-1" directly after
    // the prefix colon. Guards against a fix that always adds a space.
    let out = run("arr=(a b c); echo ${arr[@]: 1:-1}");
    assert_eq!(out.code, 1, "A3: rc must be 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(": -1: substring expression < 0"),
        "A3: stderr must contain ': -1: substring expression < 0'; got {stderr}"
    );
    assert!(
        !stderr.contains("  -1: substring expression < 0"),
        "A3: stderr must not gain a spurious space; got {stderr}"
    );
}

#[test]
fn offset_quoted_operand_keeps_trailing_space_in_token() {
    expect_diag(
        "v=abcdef; echo ${v: '1' }",
        "v: '1' : arithmetic syntax error: operand expected (error token is \"'1' \")",
        "A4 offset '1' with trailing space",
    );
}

#[test]
fn length_quoted_operand_keeps_trailing_space_in_token() {
    expect_diag(
        "v=abcdef; echo ${v: 1: '2' }",
        "v: '2' : arithmetic syntax error: operand expected (error token is \"'2' \")",
        "A5 length '2' with trailing space",
    );
}

#[test]
fn offset_quoted_operand_without_trailing_space_is_control() {
    // rubash#539 territory: no trailing blank in the source, none in the
    // token. A fix for the trailing-space face must not invent one here.
    expect_diag(
        "v=abcdef; echo ${v: '1'}",
        "v: '1': arithmetic syntax error: operand expected (error token is \"'1'\")",
        "A6 offset '1' without trailing space",
    );
}

#[test]
fn nested_default_word_negative_length_echoes_raw_text() {
    // Chained: step 1 seeds the array, step 2 expands a substring inside
    // a `:-` word; the diagnostic still echoes the raw " -1".
    expect_diag(
        "arr=(a b c); unset x; echo ${x:-${arr[@]: 1: -1}}",
        "  -1: substring expression < 0",
        "A7 nested default word negative length",
    );
}

#[test]
fn positional_negative_length_echoes_raw_text_with_leading_space() {
    expect_diag(
        "set -- a b c; echo ${@: 1: -1}",
        "  -1: substring expression < 0",
        "A8 positional negative length, raw text",
    );
}

#[test]
fn scalar_negative_length_beyond_end_echoes_raw_text() {
    expect_diag(
        "v=abcdef; echo ${v:1: -9}",
        "  -9: substring expression < 0",
        "A9 scalar length -9 adjusted below zero, raw text",
    );
}

#[test]
fn offset_operator_token_keeps_trailing_space() {
    // The token is the scanner remainder: for "1 + " GNU reports "+ "
    // (operator plus its trailing blank), and the echoed expression
    // keeps the trailing blank too ("1 + : arithmetic ...").
    expect_diag(
        "v=abcdef; echo ${v: 1 + }",
        "v: 1 + : arithmetic syntax error: operand expected (error token is \"+ \")",
        "A10 dangling operator with trailing space",
    );
}

#[test]
fn valid_offset_with_trailing_space_still_slices() {
    // Control: trailing whitespace on a *valid* offset is harmless in
    // both shells (offset 2 of abcdef).
    let out = run("v=abcdef; echo ${v: 2 }");
    assert_eq!(out.code, 0, "A11: rc must be 0");
    assert_eq!(
        out.stdout,
        b"cdef\n",
        "A11: valid offset with trailing space must slice; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "A11: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
