//! Issue unixwin/rubash#3605 (run-heredoc): the lexer's physical
//! heredoc gather end line (`Token::heredoc_end_line`) was left at its
//! fragment-relative value when a folded/reparsed region's line numbers
//! were rebased, so the EOF gather warning's reported line stayed
//! inside the fragment's own coordinate space. `f() {\ncat <<EOF\nx`
//! warned at line 2 where GNU warns at line 3 — every `position` /
//! `end_position` shift site now shifts the gather end line in
//! lockstep.
//!
//! GNU owner: make_cmd.c gather line accounting — the warning reports
//! `line_number` once the gather stops at EOF (the physical end line,
//! rubash#1028 family).
//!
//! Expectations verified byte-identical against GNU Bash 5.3.20
//! (2026-11-15): both warnings name `line {gather start}` and print
//! under the `line {gather end}` prefix.

use std::process::{Command, Stdio};

fn run_rubash_source(source: &str, tag: &str) -> (String, String, Option<i32>) {
    let path =
        std::env::temp_dir().join(format!("issue3605_hdend_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, source).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Folded `{ ...` body re-lexed at brace line 1: the body occupies
/// physical line 2, so the unterminated gather warns under the
/// `line 2` prefix naming `at line 1`. Before the rebase the fragment
/// line (1) leaked into the prefix.
#[test]
fn folded_brace_body_eof_warning_reports_gather_end_line() {
    let (_stdout, stderr, _code) = run_rubash_source("{ cat <<EOF\nx\n", "brace");
    assert!(
        stderr.contains("line 2: warning: here-document at line 1 delimited by end-of-file"),
        "stderr: {stderr}"
    );
}

/// Folded `name() { ...` body re-lexed at line 2: the gather spans
/// lines 2-3, so the warning prints under `line 3` naming `at line 2`.
#[test]
fn folded_function_body_eof_warning_reports_gather_end_line() {
    let (_stdout, stderr, _code) = run_rubash_source("f() {\ncat <<EOF\nx\n", "func");
    assert!(
        stderr.contains("line 3: warning: here-document at line 2 delimited by end-of-file"),
        "stderr: {stderr}"
    );
}

/// A brace group opened deeper in the file rebases the same way:
/// `x=1` pushes the `{` fold to line 2 and the gather to lines 3-4.
#[test]
fn offset_brace_body_eof_warning_reports_gather_end_line() {
    let (_stdout, stderr, _code) = run_rubash_source("x=1\n{\ncat <<EOF\nx\n", "offset");
    assert!(
        stderr.contains("line 4: warning: here-document at line 3 delimited by end-of-file"),
        "stderr: {stderr}"
    );
}
