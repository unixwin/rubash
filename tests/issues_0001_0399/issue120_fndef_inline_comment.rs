//! Regressions for the trailing-comment parse failure reported as a
//! follow-up on niubash #120 (rubash master 623b1f7b, niu 1.1.4).
//!
//! GNU bash accepts a `#' comment sharing the line with a function
//! definition's opening or closing brace (`f() { # note' / `f() { echo x; } #
//! note'), and the same shapes for a bare brace group. rubash rejected all of
//! them: the brace-group token swallowed the comment text, so
//! `parse_function_command` never saw a `{' body and fell through to
//! `syntax error near unexpected token `(''.
//!
//! Two lexer faults produced that:
//!   1. `has_unclosed_brace_group' did not recognise `f() {' / `function f {'
//!      as opening a group, so the logical line was not continued; the scanner
//!      then sliced from the `{' to end of line and emitted `{ # note'.
//!   2. `brace_close_can_end_compact_group' did not treat a `#' after the
//!      closing brace as terminating the group, so `{ echo x; } # note' fused
//!      into a single token.
//!
//! Every case below is pinned against GNU bash 5.3.15 (`bash -c`).

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

// ---------------------------------------------------------------------------
// Function definitions with a comment on the brace line
// ---------------------------------------------------------------------------

#[test]
fn multiline_definition_comment_after_open_brace() {
    assert_ok("f() { # note\n  echo x\n}\nf\n", "x\n");
}

#[test]
fn oneline_definition_comment_after_close_brace() {
    assert_ok("f() { echo x; } # note\nf\n", "x\n");
}

#[test]
fn oneline_definition_comment_glued_to_hash() {
    assert_ok("f() { echo x; } #note\nf\n", "x\n");
}

#[test]
fn oneline_definition_comment_after_tab() {
    assert_ok("f() { echo x; }\t# note\nf\n", "x\n");
}

#[test]
fn oneline_definition_empty_comment() {
    assert_ok("f() { echo x; } #\nf\n", "x\n");
}

#[test]
fn keyword_form_definition_comment_after_open_brace() {
    assert_ok("function f { # note\n  echo x\n}\nf\n", "x\n");
}

#[test]
fn keyword_form_oneline_definition_comment_after_close_brace() {
    assert_ok("function f { echo x; } # note\nf\n", "x\n");
}

// ---------------------------------------------------------------------------
// Bare brace group with a trailing comment
// ---------------------------------------------------------------------------

#[test]
fn brace_group_comment_after_close_brace() {
    assert_ok("{ echo x; } # note\n", "x\n");
}

#[test]
fn brace_group_arguments_survive_trailing_comment() {
    assert_ok("{ printf '%s\\n' a b; } # note\n", "a\nb\n");
}

// ---------------------------------------------------------------------------
// Controls: the same shapes without the comment must not regress, and the
// comment must not become part of a word.
// ---------------------------------------------------------------------------

#[test]
fn definition_without_comment_still_runs() {
    assert_ok("f() { echo x; }\nf\n", "x\n");
    assert_ok("f() {\n  echo x\n}\nf\n", "x\n");
}

#[test]
fn brace_still_octothorpe_inside_a_word_is_literal() {
    // `{a' is an ordinary word (`{' is a reserved word only when it stands
    // alone), so the trailing `#' must not be sliced into the brace shape.
    assert_ok("echo {a\n", "{a\n");
    assert_ok("echo a{b\n", "a{b\n");
}

#[test]
fn unterminated_group_arguments_are_not_fused() {
    // An unterminated `{' followed by a separator still stands alone as a
    // reserved word, so the following words stay separate arguments: GNU
    // `echo { a' prints `{ a' (two arguments), `echo {' prints `{'.
    assert_ok("echo { a\n", "{ a\n");
    assert_ok("echo {\n", "{\n");
}

#[test]
fn glued_hash_after_brace_is_not_a_comment() {
    // `{#note' is one word in GNU: a `#' comments only at a word start, and
    // the `{' is not standing alone. GNU rejects the definition, so this must
    // stay a parse error rather than quietly defining `f'.
    let (stdout, _stderr, code) = run_rubash("f() {#note\n  echo x\n}\nf\n");
    assert_ne!(code, Some(0), "expected the glued `{{#note' form to fail");
    assert_eq!(stdout, "");
}
