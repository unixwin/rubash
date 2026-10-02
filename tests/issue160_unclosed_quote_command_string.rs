//! niubash#160 (engine side): the `-c` command-string driver must give GNU's
//! read-time EOF diagnostics for every unterminated construct, not exit
//! silently (or execute the broken word). GNU baseline, verified against WSL
//! GNU Bash 5.3.0 (`/usr/local/bin/bash`) with a script-file driver probing
//! `-c` shapes and capturing per-shape stdout+stderr+rc (artifacts:
//! target/probe160-driver.sh run under both shells, 2026-09-28):
//!
//!   bash -c 'echo "x'   -> stderr `bash: -c: line 1: unexpected EOF while
//!                          looking for matching `"'`, no stdout, rc=2
//!   bash -c 'echo $(x'  -> `... line 2: ... matching `)'`, rc=2
//!   bash -c 'foo=([)'   -> `... line 1: ... matching `]'`, rc=1
//!   bash -c 'cat << "q' -> heredoc-delimiter quote arm, `... matching `"'`,
//!                          rc=2 (parse.y:5419-5437 read_token_word fails
//!                          the WORD read; no heredoc is gathered)
//!   multi-line `-c`     -> complete prefix lines still execute, then the
//!                          diagnostic at the open line, rc=2
//!
//! GNU owners: parse.y:5419-5437 read_token_word EOF family, parse.y:6890
//! yyerror, error.c:324 (exit status 2), shell.c parse_and_execute for -c.
//! Rubash owner: script_driver.rs run_source_impl unclosed-syntax gate.
//!
//! niubash's own `-c` route (crates/niubash-runtime shell.rs execute_script)
//! used to bypass this gate via its tokenize+parse fast path — that fix and
//! its tests live in the niubash tree. This file pins the ENGINE contract
//! the niubash route falls through to.

use std::process::Command;

fn run_c(command: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(command)
        .output()
        .expect("spawn rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// The shape from the issue report: silent exit is forbidden; GNU prints the
/// matching-`"` diagnostic and exits 2.
#[test]
fn unclosed_double_quote_reports_and_exits_2() {
    let (stdout, stderr, code) = run_c("echo \"x");
    assert_eq!(stdout, "", "no stdout: the broken word never runs");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, 2);
}

#[test]
fn unclosed_single_quote_and_backtick_report() {
    let (_, stderr, code) = run_c("echo 'x");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching `''\n"
    );
    assert_eq!(code, 2);

    let (_, stderr, code) = run_c("echo `x");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching ``'\n"
    );
    assert_eq!(code, 2);
}

#[test]
fn unclosed_command_and_parameter_substitution_report() {
    // GNU reports `$(` at the line after the open (read_token consumed the
    // newline-looking-for-`)` line).
    let (_, stderr, code) = run_c("echo $(x");
    assert_eq!(
        stderr,
        "bash: -c: line 2: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, 2);

    let (_, stderr, code) = run_c("echo ${x");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching `}'\n"
    );
    assert_eq!(code, 2);
}

/// `foo=([)` — compound-array subscript shape keeps GNU's rc=1 (parse.y
/// parse_compound_assignment EOF family; error.c:324's errexit override does
/// not apply without -e).
#[test]
fn unclosed_compound_assignment_subscript_exits_1() {
    let (_, stderr, code) = run_c("foo=([)");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching `]'\n"
    );
    assert_eq!(code, 1);
}

/// `cat << "q` — the heredoc-delimiter quote arm must fire BEFORE any
/// heredoc is gathered (parse.y:5419-5437: the WORD read fails, no
/// here-document is ever declared, so no end-of-file-delimiter warning).
#[test]
fn heredoc_delimiter_unclosed_quote_reports_matching_quote() {
    let (_, stderr, code) = run_c("cat << \"q");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, 2);
}

/// Complete commands on earlier LINES still run before the diagnostic
/// (GNU's reader parses line-by-line); commands joined by `;` on the
/// failing line do not — they are one parse unit with the broken word.
#[test]
fn complete_prefix_lines_execute_before_diagnostic() {
    let (stdout, stderr, code) = run_c("echo multi1\necho multi2\necho \"x");
    assert_eq!(stdout, "multi1\nmulti2\n");
    assert_eq!(
        stderr,
        "bash: -c: line 3: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, 2);

    let (stdout, _, code) = run_c("echo before; echo \"x");
    assert_eq!(stdout, "", "same-list prefix must not execute");
    assert_eq!(code, 2);
}

/// The optional $0 word after the command string names the diagnostic
/// prefix (error.c get_name_for_error -> $0).
#[test]
fn command_name_word_becomes_diagnostic_prefix() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"x")
        .arg("myzero")
        .output()
        .expect("spawn rubash -c with name");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "myzero: -c: line 1: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(output.status.code(), Some(2));
}

/// Silent-exit shape guard: an input that only differs by a closing quote
/// must keep executing normally through the same driver (no gate leak into
/// closed input).
#[test]
fn closed_equivalents_still_execute() {
    let (stdout, stderr, code) = run_c("echo \"x\"");
    assert_eq!(stdout, "x\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0);

    let (stdout, _, code) = run_c("echo $(echo y)");
    assert_eq!(stdout, "y\n");
    assert_eq!(code, 0);
}
