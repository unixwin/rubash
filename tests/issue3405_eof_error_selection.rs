//! Regression pins for rubash#3405 — EOF error SELECTION inside an
//! unclosed compound.
//!
//! The mechanism, from GNU source: report_syntax_error (parse.y:6833)
//! exits at a concrete token (parse.y:6852-6865: current_token != 0
//! with EOF_Reached == 0) before the EOF branch (parse.y:6890-6901)
//! can report an open compound, so the first token-level verdict in
//! the region — a dangling redirection whose target never arrives
//! (parse.y:532 `redirection: '>' WORD`), a mismatched closer, a
//! token error inside a command-substitution body (reported with the
//! `while looking for matching \`)'` suffix of parse.y:6858-6859,
//! dropped when the offending token IS the substitution's eof token)
//! — precedes every "unexpected end of file" report. Only a parse
//! that survives to exhaustion takes the EOF form.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (lane dossier probes.py E-series, oracle-verified).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3405-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn expect(root: &Path, body: &str, rc: i32, stdout: &str, stderr: &str) {
    let out = run(root, body);
    assert_eq!(out.rc, rc, "body: {body:?}");
    assert_eq!(out.stdout, stdout, "body: {body:?}");
    assert_eq!(out.stderr, stderr, "body: {body:?}");
}

/// Shape 1 (issue body): a dangling redirect inside an unclosed
/// function brace group — the near-`newline` verdict at the
/// operator's line, not the unclosed-`{` EOF form.
#[test]
fn eval_dangling_redirect_in_brace_group_names_newline() {
    let root = scratch("e01");
    expect(
        &root,
        "eval 'X() { (a)>'\necho rc=$?\n",
        0,
        "rc=2\n",
        "case.sh: eval: line 1: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 1: `X() { (a)>'\n",
    );
}

/// Shape 2 (issue body): same family, bare operator.
#[test]
fn eval_bare_dangling_redirect_in_brace_group_names_newline() {
    let root = scratch("e02");
    expect(
        &root,
        "eval 'g() { >'\necho rc=$?\n",
        0,
        "rc=2\n",
        "case.sh: eval: line 1: syntax error near unexpected token `newline'\n\
         case.sh: eval: line 1: `g() { >'\n",
    );
}

/// A dangling redirect inside an unclosed subshell names `newline`
/// at the operator's line (the `(' EOF form loses).
#[test]
fn dangling_redirect_in_subshell_names_newline() {
    let root = scratch("e09");
    expect(
        &root,
        "( (a)>\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `( (a)>'\n",
    );
}

/// Same family in a brace group (file mode).
#[test]
fn dangling_redirect_in_brace_group_names_newline() {
    let root = scratch("e10");
    expect(
        &root,
        "{ (a)>\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `{ (a)>'\n",
    );
}

/// Same family in a `while` body: the keyword EOF arm yields to the
/// token-level verdict.
#[test]
fn dangling_redirect_in_while_body_names_newline() {
    let root = scratch("e11");
    expect(
        &root,
        "while true; do >\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `while true; do >'\n",
    );
}

/// Same family in an `if` body.
#[test]
fn dangling_redirect_in_if_body_names_newline() {
    let root = scratch("e20");
    expect(
        &root,
        "if true; then >\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `if true; then >'\n",
    );
}

/// Shape 3 (issue body): a dangling redirect inside an unclosed
/// command substitution — the comsub body's own yyparse dies on the
/// newline token, reported with the matching suffix at line 1.
#[test]
fn dangling_redirect_in_comsub_names_newline_with_matching_suffix() {
    let root = scratch("e03");
    expect(
        &root,
        "x=$( ( a) >>\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline' while looking for matching `)'\n\
         case.sh: line 1: `x=$( ( a) >>'\n",
    );
}

/// Same family, single `>`.
#[test]
fn dangling_redirect_in_comsub_single_operator() {
    let root = scratch("e17");
    expect(
        &root,
        "x=$( ( a) >\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline' while looking for matching `)'\n\
         case.sh: line 1: `x=$( ( a) >'\n",
    );
}

/// A bare `(` after comsub body text is a body token error too
/// (parse.y:6858-6859), with the matching suffix.
#[test]
fn bare_paren_after_comsub_body_text_names_newline_with_matching_suffix() {
    let root = scratch("e08");
    expect(
        &root,
        "x=$(echo (\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline' while looking for matching `)'\n\
         case.sh: line 1: `x=$(echo ('\n",
    );
}

/// The eval face of shape 3: a token error inside the comsub body
/// aborts through parse.y:4588-4596 (FORCE_EOF) — the script dies
/// with status 1 and the following command never runs.
#[test]
fn eval_dangling_redirect_in_comsub_aborts_script() {
    let root = scratch("e15");
    expect(
        &root,
        "eval 'x=$( ( a) >>'\necho rc=$?\n",
        1,
        "",
        "case.sh: eval: line 1: syntax error near unexpected token `newline' while looking for matching `)'\n\
         case.sh: eval: line 1: `x=$( ( a) >>'\n",
    );
}

/// When the token that fails the dangling redirect IS the
/// substitution's eof token, GNU names it and drops the matching
/// suffix (parse.y:6858: current_token == shell_eof_token).
#[test]
fn comsub_close_paren_as_redirect_target_names_paren_without_suffix() {
    let root = scratch("e21");
    expect(
        &root,
        "x=$( ( a) >> )\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `)'\n\
         case.sh: line 1: `x=$( ( a) >> )'\n",
    );
}

/// Control: a comsub whose body parses cleanly to exhaustion keeps
/// the plain matching-EOF form (no token error exists to preempt).
#[test]
fn clean_comsub_body_keeps_matching_eof() {
    let root = scratch("e07");
    expect(
        &root,
        "x=$( ( a)\n",
        2,
        "",
        "case.sh: line 2: unexpected EOF while looking for matching `)'\n",
    );
}

/// Control: an unclosed `if` with no dangling redirect keeps the
/// compound EOF form.
#[test]
fn unclosed_if_without_redirect_keeps_compound_eof() {
    let root = scratch("e06");
    expect(
        &root,
        "if true; then\n",
        2,
        "",
        "case.sh: line 2: syntax error: unexpected end of file from `if' command on line 1\n",
    );
}

/// Control: a complete function definition whose dangling redirect
/// meets a real `;` keeps the long-standing near-`;` verdict.
#[test]
fn complete_brace_group_dangling_redirect_names_semicolon() {
    let root = scratch("e12");
    expect(
        &root,
        "X() { (a)> ; }\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `;'\n\
         case.sh: line 1: `X() { (a)> ; }'\n",
    );
}

/// Control: a top-level dangling redirect (no compound at all)
/// keeps its near-`newline` verdict.
#[test]
fn toplevel_dangling_redirect_names_newline() {
    let root = scratch("e19");
    expect(
        &root,
        "(a) >\n",
        2,
        "",
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `(a) >'\n",
    );
}

/// Control: an unclosed brace group with no token-level error in
/// its region keeps the compound EOF form (eval face).
#[test]
fn eval_unclosed_brace_without_redirect_keeps_compound_eof() {
    let root = scratch("e22");
    expect(
        &root,
        "eval 'X() { (a)'\necho rc=$?\n",
        0,
        "rc=2\n",
        "case.sh: eval: line 2: syntax error: unexpected end of file from `{' command on line 1\n",
    );
}

/// Control (the rubash#3183-locked exception): a continuation
/// pending at end of input suppresses the dangling verdict — the
/// newline that would offend was elided by the trailing backslash.
#[test]
fn eval_continuation_at_eof_keeps_compound_eof() {
    let root = scratch("e16");
    expect(
        &root,
        "eval 'X() { (a)>\\'\n: < cve7169-bad2\n",
        1,
        "",
        "case.sh: eval: line 3: syntax error: unexpected end of file from `{' command on line 1\n\
         case.sh: line 2: cve7169-bad2: No such file or directory\n",
    );
}

/// Control (issue shape 4, re-verified): an unclosed backquote
/// inside `case` is a matched-pair scan exhaustion — the EOF form
/// stands on BOTH sides; the near-token arm must not fire on text
/// the reader never tokenized.
#[test]
fn unclosed_backquote_in_case_keeps_matching_eof() {
    let root = scratch("e04");
    expect(
        &root,
        "case x in `\n",
        2,
        "",
        "case.sh: line 1: unexpected EOF while looking for matching ``'\n",
    );
}

/// Control: a heredoc pending inside an unclosed brace group keeps
/// its warning + compound EOF pair.
#[test]
fn heredoc_in_unclosed_brace_keeps_warning_and_compound_eof() {
    let root = scratch("e14");
    expect(
        &root,
        "X() { cat <<EOF\n",
        2,
        "",
        "case.sh: line 1: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n\
         case.sh: line 2: syntax error: unexpected end of file from `{' command on line 1\n",
    );
}
