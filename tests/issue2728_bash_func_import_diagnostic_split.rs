//! rubash#2728 (B5-04): exported-function (`BASH_FUNC_*%%`) import
//! must split its failures the way GNU does.
//!
//! GNU anchor: variables.c initialize_shell_variables — the import
//! only engages when the value starts with `() {` (STREQN 4); any
//! other value is bound as an ordinary variable, silently. An engaged
//! value is rebuilt as `name value` and run through
//! parse_and_execute (SEVAL_FUNCDEF|SEVAL_ONECMD, builtins/
//! evalstring.c):
//!   - a parse failure prints the parser's own diagnostics
//!     (`f: line 0: syntax error near unexpected token ...` plus the
//!     echoed definition line), then variables.c adds the single
//!     `error importing function definition for `f'` line;
//!   - a clean parse that is not one straight function definition
//!     (trailing same-line command — the ShellShock shape) prints
//!     `warning: f: ignoring function definition attempt` plus the
//!     same error line, with no parser diagnostics;
//!   - either failure leaves `$?` = 1 (EXECUTION_FAILURE) for the
//!     first command; silent skips leave it 0.
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

fn run_import(value: &str, script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("BASH_FUNC_f%%", value)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn shell_name() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

const DEFINED_PROBE: &str = "declare -F f >/dev/null && echo DEFINED || echo UNDEF";

#[test]
fn non_function_value_is_ignored_silently() {
    let (stdout, stderr) = run_import(
        "not a func",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    assert_eq!(stdout, "st=0\nUNDEF\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn nospace_brace_form_is_rejected_silently() {
    // GNU's prefix check is STREQN("() {", string, 4): `(){` never
    // engages the import, so the function stays undefined with no
    // diagnostic at all (rubash used to ACCEPT this shape).
    let (stdout, stderr) = run_import(
        "(){ echo OK; }",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    assert_eq!(stdout, "st=0\nUNDEF\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn leading_space_value_is_rejected_silently() {
    let (stdout, stderr) = run_import(" () { echo OK; }", DEFINED_PROBE);
    assert_eq!(stdout, "UNDEF\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn trailing_garbage_reports_parser_diagnostic_then_error_line() {
    let (stdout, stderr) = run_import(
        "() { echo OK; } garbage",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    let sh = shell_name();
    assert_eq!(stdout, "st=1\nUNDEF\n");
    assert_eq!(
        stderr,
        format!(
            "{sh}: f: line 0: syntax error near unexpected token `garbage'\n\
             {sh}: f: line 0: `f () {{ echo OK; }} garbage'\n\
             {sh}: error importing function definition for `f'\n"
        ),
        "stderr: {stderr}"
    );
}

#[test]
fn malformed_body_reports_parser_diagnostic_then_error_line() {
    let (stdout, stderr) = run_import(
        "() { if; }",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    let sh = shell_name();
    assert_eq!(stdout, "st=1\nUNDEF\n");
    assert_eq!(
        stderr,
        format!(
            "{sh}: f: line 0: syntax error near unexpected token `;'\n\
             {sh}: f: line 0: `f () {{ if; }}'\n\
             {sh}: error importing function definition for `f'\n"
        ),
        "stderr: {stderr}"
    );
}

#[test]
fn shellshock_shape_warns_without_parser_diagnostic() {
    let (stdout, stderr) = run_import(
        "() { echo OK; }; echo PWNED",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    let sh = shell_name();
    assert_eq!(stdout, "st=1\nUNDEF\n");
    assert_eq!(
        stderr,
        format!(
            "{sh}: warning: f: ignoring function definition attempt\n\
             {sh}: error importing function definition for `f'\n"
        ),
        "stderr: {stderr}"
    );
}

#[test]
fn unclosed_body_reports_eof_diagnostic_then_error_line() {
    let (stdout, stderr) = run_import(
        "() { echo OK;",
        "echo st=$?; declare -F f >/dev/null && echo DEFINED || echo UNDEF",
    );
    let sh = shell_name();
    assert_eq!(stdout, "st=1\nUNDEF\n");
    assert_eq!(
        stderr,
        format!(
            "{sh}: f: line 1: syntax error: unexpected end of file from `{{' command on line 0\n\
             {sh}: error importing function definition for `f'\n"
        ),
        "stderr: {stderr}"
    );
}

#[test]
fn valid_definition_imports_and_runs() {
    let (stdout, stderr) = run_import("() { echo OK; }", "f");
    assert_eq!(stdout, "OK\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn trailing_separator_and_redirect_still_define() {
    let (stdout, stderr) = run_import("() { echo OK; };", DEFINED_PROBE);
    assert_eq!(stdout, "DEFINED\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
    let (stdout, stderr) = run_import("() { echo OK; } 2>/dev/null", DEFINED_PROBE);
    assert_eq!(stdout, "DEFINED\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn newline_separated_command_is_never_read() {
    // SEVAL_ONECMD: parse_and_execute stops after the first command;
    // a command on a LATER line of the value is not even read, so the
    // definition stands and nothing else runs.
    let (stdout, stderr) = run_import("() { echo OK; }\necho PWNED", DEFINED_PROBE);
    assert_eq!(stdout, "DEFINED\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn multiline_body_imports() {
    let (stdout, stderr) = run_import("() { echo a;\necho b; }", "f");
    assert_eq!(stdout, "a\nb\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}
