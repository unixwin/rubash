//! rubash#3085 — `$'...'` spans in array subscripts reach the arithmetic
//! evaluator raw, leaking the `$` (and, on the read path, the lexer's
//! carrier byte) into the error token. GNU decodes the span first, and
//! the two paths then differ by design:
//!
//! - Write path (assignment subscript): the evaluator receives the
//!   decoded content re-quoted with single quotes (GNU subst.c:4133
//!   `expand_string_dollar_quote`, `sh_single_quote` at subst.c:4231 /
//!   4253), so the token reads `'x y'` — and even `'1'` errors, because
//!   a quoted literal is not an arithmetic operand.
//! - Read path: the evaluator receives the bare decoded content, so
//!   `1+2` evaluates and `x y` errors on the trailing input `y`.
//! - Associative subscripts are string keys: decoded bare, both ways.
//!
//! Plain `'...'` without the `$` is the control: GNU's write-path token
//! is byte-identical to the `$'...'` one ($' ≡ ' after decode).
//!
//! Expectations byte-measured against GNU bash 5.3.20. Harness mirrors
//! issue3024: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`.

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

/// Fatal subscript diagnostic: rc 1, empty stdout (the -c run aborts on
/// the expansion error before any following command), stderr needle.
fn expect_token_error(script: &str, needle: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        1,
        "{ctx}: rc must be 1; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{ctx}: stdout must be empty (fatal error aborts the run); got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
    assert!(
        !stderr.contains("$'"),
        "{ctx}: stderr must not leak the raw $' span; got {stderr}"
    );
}

fn expect_value(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        expected_stdout,
        "{ctx}: stdout mismatch; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn write_dollar_single_quote_token_is_decoded_and_requoted() {
    expect_token_error(
        "unset a; a[$'x y']=b; echo \"rc=$?\"",
        "'x y': arithmetic syntax error: operand expected (error token is \"'x y'\")",
        "C1 write a[$'x y']",
    );
}

#[test]
fn write_mixed_spans_token_is_decoded_and_requoted() {
    expect_token_error(
        "unset a; a[$'x'$\"y\"]=b; echo \"rc=$?\"",
        "'x'y: arithmetic syntax error: operand expected (error token is \"'x'y\")",
        "C2 write a[$'x'$\"y\"]",
    );
}

#[test]
fn write_dollar_single_quote_comsub_text_expands_then_requotes() {
    // The comsub text inside $'...' is expanded first (to `1+2`), then
    // the product is re-quoted: token '1+2', not the raw source text.
    expect_token_error(
        "unset a; a[$'$(echo 1+2)']=b; echo \"rc=$?\"",
        "'1+2': arithmetic syntax error: operand expected (error token is \"'1+2'\")",
        "C3 write a[$'$(echo 1+2)']",
    );
}

#[test]
fn read_dollar_single_quote_arith_evaluates() {
    expect_value(
        "a=(zero one two three); printf '%s\\n' \"${a[$'1+2']}\"",
        b"three\n",
        "C4 read a[$'1+2']",
    );
}

#[test]
fn read_dollar_single_quote_bad_name_token_is_bare() {
    // Read path decodes to bare `x y`: the evaluator reads variable x
    // (= 0), then errors on the trailing input y.
    expect_token_error(
        "a=(zero one two three); echo \"${a[$'x y']}\"; echo \"rc=$?\"",
        "x y: arithmetic syntax error in expression (error token is \"y\")",
        "C5 read a[$'x y']",
    );
}

#[test]
fn write_plain_single_quote_is_the_control() {
    // Control: without the $, GNU produces the identical diagnostic —
    // this is the invariant the $' decode must reproduce.
    expect_token_error(
        "unset a; a['x y']=b; echo \"rc=$?\"",
        "'x y': arithmetic syntax error: operand expected (error token is \"'x y'\")",
        "C6 write a['x y'] (control)",
    );
}

#[test]
fn write_dollar_single_quote_numeric_still_operand_error() {
    // The write path re-quotes even a numeric decode: '1' is a quoted
    // literal, not an operand, so GNU errors here too.
    expect_token_error(
        "unset a; a[$'1']=b; echo \"rc=$?\"; declare -p a",
        "'1': arithmetic syntax error: operand expected (error token is \"'1'\")",
        "C7 write a[$'1']",
    );
}

#[test]
fn assoc_write_dollar_single_quote_key_decodes_bare() {
    // Chained: the assignment must land under the decoded key, proven
    // by the declare dump on the next line.
    let out = run("declare -A m; m[$'x y']=b; echo \"rc=$?\"; declare -p m");
    assert_eq!(
        out.code,
        0,
        "C8: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rc=0"),
        "C8: assignment must succeed; stdout: {stdout}"
    );
    assert!(
        stdout.contains("[\"x y\"]=\"b\""),
        "C8: declare must show the decoded key; stdout: {stdout}"
    );
    assert!(
        out.stderr.is_empty(),
        "C8: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn assoc_read_dollar_single_quote_key_decodes_bare() {
    expect_value(
        "declare -A m=([x y]=hit); echo \"${m[$'x y']}\"",
        b"hit\n",
        "C9 assoc read m[$'x y']",
    );
}

#[test]
fn read_dollar_single_quote_escape_decodes() {
    // \x2b is '+': the decoded subscript is 1+2, as in C4.
    expect_value(
        "a=(zero one two three); printf '%s\\n' \"${a[$'1\\x2b2']}\"",
        b"three\n",
        "C10 read a[$'1\\x2b2']",
    );
}
