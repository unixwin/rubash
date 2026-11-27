//! rubash#3184 regressions: EOF / error attribution when an `if`
//! construct (condition, then/elif/else body, elif condition) contains
//! an arithmetic command.
//!
//! Two defects, one family:
//!
//! 1. When `parse_if_command` fails and the caller falls back to
//!    `if_frame_offender` (the resync scan that decides between a
//!    mismatched-closer report and the EOF report), the scan treated
//!    the lexer's folded arithmetic-command token — one Keyword token
//!    carrying the whole `((…))` text — as an illegal phase keyword
//!    and blamed IT. An unclosed `if` whose condition or body holds a
//!    `((…))` therefore reported `syntax error near unexpected token
//!    `((1))'` instead of GNU's `syntax error: unexpected end of file
//!    from `if' command on line 1`. GNU's grammar accepts ARITH_CMD
//!    wherever a command is accepted (parse.y:1203 arith_command), so
//!    the scan now treats the folded token as the phase's command.
//!
//! 2. When a section of the `if` (condition, then/elif/else body,
//!    elif condition) errored and the boundary recovery
//!    (`find_if_then` / `scan_if_section_end`) could not place a
//!    closer, the already-latched body error was discarded and the
//!    caller's resync guessed a different token. GNU's
//!    `report_syntax_error` (parse.y:6841) reports the FIRST offending
//!    token in stream order — yyerror (parse.y:6724) fires where the
//!    grammar actually failed — so an argument-position `((` inside
//!    the construct names `(` (the lexer returns the bare `(` there:
//!    `parse_dparen`, parse.y:4895, is gated by
//!    `reserved_word_acceptable`, parse.y:4926, which an argument
//!    position fails), never an earlier `((…))` and never the EOF
//!    form. The if parser now returns the latched error itself when
//!    recovery fails.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle). On the unfixed tree
//! every defect cell fails (EOF cells name the arithmetic command;
//! error cells name `((0))` or fall back to the EOF form); every
//! control passes on both trees.
#![cfg(unix)]

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3184-if-{}-{}",
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

/// An unclosed `if`: GNU reports the EOF form at the physical EOF
/// line, naming the `if` and its opening line; no source echo, rc 2.
fn check_eof_from_if(script: &str, eof_line: usize) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert_eq!(
        stderr,
        format!(
            "case.sh: line {eof_line}: syntax error: unexpected end of file from `if' command on line 1\n"
        ),
        "script: {script:?}"
    );
}

/// A real token error inside the construct: GNU names the bare `(`
/// of the argument-position `((`, echoes the offending line, rc 2.
fn check_near_paren(script: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    let line = script.lines().next().unwrap_or_default();
    assert_eq!(
        stderr,
        format!(
            "case.sh: line 1: syntax error near unexpected token `('\ncase.sh: line 1: `{line}'\n"
        ),
        "script: {script:?}"
    );
}

// ---- Defect cells: unclosed `if` with an arithmetic command ----

#[test]
fn unclosed_if_arith_condition_and_body_reports_eof_from_if() {
    check_eof_from_if("if ((1))\nthen ((2))\n", 3);
}

#[test]
fn unclosed_if_arith_no_trailing_newline_reports_eof_from_if() {
    check_eof_from_if("if ((1)); then ((2))", 2);
}

#[test]
fn unclosed_if_arith_condition_simple_body_reports_eof_from_if() {
    check_eof_from_if("if ((1)); then echo a\n", 2);
}

#[test]
fn unclosed_if_simple_condition_arith_body_reports_eof_from_if() {
    check_eof_from_if("if true; then ((2))\n", 2);
}

#[test]
fn unclosed_if_cond_command_arith_body_reports_eof_from_if() {
    check_eof_from_if("if [[ x ]]; then ((2))\n", 2);
}

#[test]
fn unclosed_if_elif_arith_condition_reports_eof_from_if() {
    check_eof_from_if("if ((1)); then echo a; elif ((2)); then echo b\n", 2);
}

#[test]
fn unclosed_if_arith_condition_alone_reports_eof_from_if() {
    check_eof_from_if("if ((1))\n", 2);
}

#[test]
fn unclosed_if_spaced_arith_reports_eof_from_if() {
    check_eof_from_if("if (( 1 + 2 )); then (( 3 ))\n", 2);
}

// ---- Defect cells: argument-position `((` inside the construct ----

#[test]
fn missing_separator_elif_names_argument_paren_not_condition_arith() {
    check_near_paren("if ((0)) then echo a elif ((2)) then echo b else echo c fi\n");
}

#[test]
fn body_argument_arith_before_then_names_argument_paren() {
    check_near_paren("if ((0)) then echo a foo ((2)) then echo b; fi\n");
}

#[test]
fn condition_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("if echo a ((2))\n");
}

#[test]
fn body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("if ((0)) then echo a ((2))\n");
}

#[test]
fn elif_condition_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("if x; then y; elif echo a ((2))\n");
}

#[test]
fn else_body_argument_arith_at_eof_names_argument_paren() {
    check_near_paren("if ((0)) then echo a; else echo b ((2))\n");
}

// ---- Controls: already correct on the unfixed tree ----

#[test]
fn unclosed_if_cond_bodies_report_eof_from_if_control() {
    check_eof_from_if("if [[ x ]]\nthen [[ y ]]\n", 3);
}

#[test]
fn unclosed_if_plain_body_reports_eof_from_if_control() {
    check_eof_from_if("if true; then echo a\n", 2);
}

#[test]
fn unclosed_while_arith_reports_eof_from_while_control() {
    let (stdout, stderr, code) = rubash_file("while ((0))\ndo echo x\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "case.sh: line 3: syntax error: unexpected end of file from `while' command on line 1\n"
    );
}

#[test]
fn body_argument_arith_with_closer_names_argument_paren_control() {
    check_near_paren("if ((0)) then echo a ((2)); fi\n");
}

#[test]
fn body_argument_arith_elif_word_with_closer_names_argument_paren_control() {
    check_near_paren("if ((0)) then echo a elif ((2)); fi\n");
}

#[test]
fn closed_if_arith_condition_and_body_still_runs() {
    let (stdout, stderr, code) = rubash_file("if ((1)); then ((2)); fi\necho after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
}

#[test]
fn closed_if_elif_arith_still_runs() {
    let (stdout, stderr, code) = rubash_file("if ((0)) then echo a; elif ((2)) then echo b; fi\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "b\n");
    assert_eq!(stderr, "");
}
