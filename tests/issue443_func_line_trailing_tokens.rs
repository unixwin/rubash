//! Regressions for unixwin/rubash#443: a function definition whose `f() {'
//! line carries a trailing comment — or opens the body on the brace line —
//! used to die with `syntax error near unexpected token `('` in the build
//! the issue reports (rubash b4cbcadc, niu 1.1.2).
//!
//! The lexer faults behind it (the brace-group token swallowing the comment
//! and `has_unclosed_brace_group' not continuing the logical line) were
//! fixed with the niubash #120 follow-up; `tests/issue120_fndef_inline_comment.rs'
//! pins the comment-after-brace shapes. This file pins the REST of #443's
//! measured scope table: bodies that start on the `{' line and close on a
//! later line, assignment-prefixed bodies, one-liner `f() { :; }' shapes,
//! tokens after the closing brace, and the control rows that were already
//! correct. Every expectation matches GNU bash 5.x.

use std::process::Command;

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
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

fn assert_ok(script: &str, expected: &str) {
    let (stdout, stderr, code) = run_rubash(script);
    assert_eq!(
        stdout, expected,
        "stdout mismatch for {script:?} (rc={code:?}, stderr={stderr:?})"
    );
    assert_eq!(
        code,
        Some(0),
        "rc mismatch for {script:?} (stderr={stderr:?})"
    );
}

fn assert_parse_error(script: &str, offending_word: &str, echoed_line: &str) {
    let (stdout, stderr, code) = run_rubash(script);
    assert_eq!(
        code,
        Some(2),
        "rc mismatch for {script:?} (stderr={stderr:?})"
    );
    assert_eq!(
        stdout, "",
        "no output may precede the parse error: {script:?}"
    );
    let stderr = stderr.replace("\"", "'");
    assert!(
        stderr.contains(&format!(
            "syntax error near unexpected token `{offending_word}'"
        )),
        "error must name the offending token `{offending_word}`: {stderr:?}"
    );
    // GNU parse.y print_offending_line echoes the physical offending line
    // verbatim — not a mangled `f ( ) {` re-rendering.
    assert!(
        stderr.contains(echoed_line),
        "error must echo the offending source line {echoed_line:?}: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// #443 scope table: body opens on the `f() {' line, `}' on a later line
// ---------------------------------------------------------------------------

#[test]
fn body_starts_on_brace_line_closes_later() {
    assert_ok("f() { echo hi\n}\nf\n", "hi\n");
}

#[test]
fn body_and_trailing_comment_on_brace_line() {
    assert_ok("f() { echo hi # c\n}\nf\n", "hi\n");
}

#[test]
fn assignment_and_comment_on_brace_line() {
    assert_ok("f() { x=1 # c\n  echo \"$x\"\n}\nf\n", "1\n");
}

#[test]
fn empty_comment_after_open_brace_multiline_body() {
    assert_ok("f() { #\n  echo hi\n}\nf\n", "hi\n");
}

// ---------------------------------------------------------------------------
// One-liner definitions: `f() { :; }` with and without a trailing comment
// ---------------------------------------------------------------------------

#[test]
fn oneliner_definition_baseline() {
    assert_ok("f() { :; }\nf\n", "");
}

#[test]
fn oneliner_definition_with_trailing_comment() {
    // The exact one-liner shape named in the issue text.
    assert_ok("f() { :; } # comment\nf\n", "");
}

#[test]
fn oneliner_comment_does_not_swallow_following_lines() {
    assert_ok("f() { :; } # comment\necho after\n", "after\n");
}

// ---------------------------------------------------------------------------
// Tokens after the closing brace
// ---------------------------------------------------------------------------

#[test]
fn comment_after_closing_brace_on_own_line() {
    assert_ok("f() {\n  echo hi\n} # trailing\nf\n", "hi\n");
}

#[test]
fn comment_after_oneliner_closing_brace() {
    assert_ok("f() { echo hi; } # trailing\nf\n", "hi\n");
}

#[test]
fn bare_word_after_multiline_closing_brace_is_syntax_error() {
    // GNU: `}' ends the definition; a bare word after it is where the
    // grammar demands a separator, so the yacc error names THAT word.
    assert_parse_error("f() {\n  echo hi\n} extra\n", "extra", "} extra");
}

#[test]
fn bare_word_after_oneliner_closing_brace_is_syntax_error() {
    // `f() { :; } echo extra' — GNU rejects the whole input before running
    // anything (rc=2, EX_BADUSAGE), naming `echo'.
    assert_parse_error(
        "f() { :; } echo extra\nf\n",
        "echo",
        "f() { :; } echo extra",
    );
}

// ---------------------------------------------------------------------------
// Controls: the rows #443 measured as already correct must stay correct
// ---------------------------------------------------------------------------

#[test]
fn multiline_definition_brace_alone() {
    assert_ok("f() {\n  echo hi\n}\nf\n", "hi\n");
}

#[test]
fn complete_oneliner_definition() {
    assert_ok("f() { echo hi; }\nf\n", "hi\n");
}

#[test]
fn name_then_brace_on_its_own_line() {
    assert_ok("f()\n{\n  echo hi\n}\nf\n", "hi\n");
}

#[test]
fn comment_with_parentheses_on_brace_line() {
    // The `(' inside a comment is reader noise — the original failure was
    // reported against `syntax error near unexpected token `('`.
    assert_ok("f() { # usage (v2)\n  echo hi\n}\nf\n", "hi\n");
}

// ---------------------------------------------------------------------------
// No interference with the assignment-prefix + subshell parse (rubash#453):
// a definition body may itself use an assignment prefix before a subshell,
// with a trailing comment on the brace line.
// ---------------------------------------------------------------------------

#[test]
fn body_assignment_prefix_subshell_with_brace_line_comment() {
    // `x=2' is the subshell's environment only: the prefix prints `2' inside
    // it, and the enclosing body still sees `1' (rubash#453 semantics).
    assert_ok(
        "f() { x=1 # c\n  x=2 (echo \"$x\")\n  echo \"$x\"\n}\nf\n",
        "2\n1\n",
    );
}
