//! rubash#3936 regressions: error attribution when a `while`/`until`
//! construct loses its `done` (or its `do`) while a section already
//! carries a parse error.
//!
//! Same family as rubash#3184 defect B, different file: `if` kept its
//! latched section errors via the Fix B pattern (PR #3934); the loop
//! parser discarded them in three places in
//! `src/parser/loop_command.rs`:
//!
//! 1. `parse_loop_body(...)?` — the `?` dropped the whole parse when
//!    no `done` was reached, without ever parsing the body slice, so a
//!    body error (an argument-position `((`, whose lexer returns the
//!    bare `(`) never existed and the caller guessed the EOF-from-
//!    `while' shape.
//! 2. Same `?`, with the error in the CONDITION: the condition slice
//!    was parsed, but its latched error sat in the discarded `condition`
//!    vec while the caller guessed the EOF shape.
//! 3. `find_loop_do(...)?` — with no `do` at all the condition list
//!    was never parsed, so a condition error was never produced.
//!
//! GNU's `report_syntax_error` (parse.y:6841) reports the FIRST
//! offending token in stream order — yyerror (parse.y:6724) fires
//! where the grammar actually failed — so an argument-position `((`
//! inside the construct names `(` (the lexer returns the bare `(`:
//! `parse_dparen`, parse.y:4895, gated by
//! `reserved_word_acceptable`, parse.y:4926), never the EOF form.
//! The loop parser now returns the latched error itself when recovery
//! fails, in stream order (condition before body); with no latched
//! error the None semantics — and the caller's EOF /
//! mismatched-closer shape — are unchanged.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.9
//! script-file probes (server oracle). On the unfixed tree every
//! defect cell falls back to the EOF-from-`while' / `until' form (or,
//! with a foreign closer, names that closer); every control passes on
//! both trees.
#![cfg(unix)]

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3936-loop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// An unclosed `while`/`until` with no latched error: GNU reports the
/// EOF form at the physical EOF line, naming the opener and its
/// opening line; no source echo, rc 2.
fn check_eof_from_loop(script: &str, opener: &str, eof_line: usize) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert_eq!(
        stderr,
        format!(
            "case.sh: line {eof_line}: syntax error: unexpected end of file from `{opener}' command on line 1\n"
        ),
        "script: {script:?}"
    );
}

/// A real token error inside the construct: GNU names the bare `(` of
/// the argument-position `((`, echoes the offending line, rc 2.
fn check_near_paren(script: &str) {
    check_near_paren_on_line(script, 1);
}

fn check_near_paren_on_line(script: &str, error_line: usize) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    let line = script.lines().nth(error_line - 1).unwrap_or_default();
    assert_eq!(
        stderr,
        format!(
            "case.sh: line {error_line}: syntax error near unexpected token `('\ncase.sh: line {error_line}: `{line}'\n"
        ),
        "script: {script:?}"
    );
}

// ---- Defect cells: body error, no `done` ----

#[test]
fn body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("while x; do echo a ((2))\n");
}

#[test]
fn missing_separator_body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("while ((0)) do echo a ((2))\n");
}

#[test]
fn until_body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("until x; do echo a ((2))\n");
}

#[test]
fn until_missing_separator_body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("until ((0)) do echo a ((2))\n");
}

#[test]
fn body_argument_arith_at_eof_no_trailing_newline_names_argument_paren() {
    check_near_paren("while x; do echo a ((2))");
}

#[test]
fn body_argument_arith_on_second_line_at_eof_names_argument_paren() {
    check_near_paren_on_line("while x; do\necho a ((2))\n", 2);
}

#[test]
fn nested_loop_body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("while x; do while y; do echo a ((2)); done\n");
}

#[test]
fn body_argument_arith_before_foreign_closer_names_argument_paren() {
    check_near_paren("while x; do echo a ((2)); fi\n");
}

// ---- Defect cells: condition error, no `do` / no `done` ----

#[test]
fn condition_argument_arith_without_do_names_argument_paren() {
    check_near_paren("while echo a ((2))\n");
}

#[test]
fn condition_argument_arith_without_done_names_argument_paren() {
    check_near_paren("while echo a ((2)) do echo hi\n");
}

// ---- Controls: already correct on the unfixed tree ----

#[test]
fn unclosed_while_plain_body_reports_eof_from_while_control() {
    check_eof_from_loop("while x; do echo a\n", "while", 2);
}

#[test]
fn unclosed_while_arith_body_reports_eof_from_while_control() {
    check_eof_from_loop("while x; do echo a; ((2))\n", "while", 2);
}

#[test]
fn unclosed_until_plain_body_reports_eof_from_until_control() {
    check_eof_from_loop("until x; do echo a\n", "until", 2);
}

#[test]
fn body_argument_arith_with_done_names_argument_paren_control() {
    check_near_paren("while x; do echo a ((2)); done\n");
}

#[test]
fn condition_argument_arith_with_done_names_argument_paren_control() {
    check_near_paren("while echo a ((2)) do echo hi; done\n");
}

#[test]
fn missing_separator_body_argument_arith_with_done_names_argument_paren_control() {
    check_near_paren("while ((0)) do echo a ((2)); done\n");
}

#[test]
fn closed_while_false_condition_still_runs() {
    let (stdout, stderr, code) = rubash_file("while ((0)); do echo a; done\necho after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
}

#[test]
fn closed_until_true_condition_still_runs() {
    let (stdout, stderr, code) = rubash_file("until ((1)); do echo a; done\necho after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
}
