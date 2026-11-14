//! Regression pins for rubash#3183 — diagnostic LINE accounting.
//!
//! The mechanism, from GNU source: execute_cmd.c:854 runs
//! do_redirections BEFORE the dispatch switch stamps the command's
//! own line, and error_prolog reads the global line_number — so a
//! compound command's own redirect-open failure reports the AMBIENT
//! line (the line the enclosing frame pinned: a top-level compound's
//! parse-end line, a subshell's `)' line, a function's definition
//! frame), not the compound's first line. The exceptions are
//! cm_cond / cm_arith / cm_arith_for, whose executing_line_number
//! (execute_cmd.c:412-428) substitutes the node's own line — with
//! cm_arith / cm_cond stamped by the parser at the CLOSING token, so
//! a construct completed on a later physical line reports that later
//! line, while cm_arith_for keeps the `for' keyword's line.
//!
//! EOF diagnostics: when a compound's parse reaches end of input,
//! the reader has consumed the input's last line — a trailing line
//! continuation means the offending construct's error reports the
//! line AFTER the last physical line (the string-input exhaustion
//! bump, parse.y string_to_string region), for every compound arm.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8, measured against the oracle
//! binary (lane dossier probe batteries: runall.sh shapes C5/C6/C7a,
//! probe-3183-lines{,2,3,4}.sh, probe-3183-eval.sh).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");
const BAD: &str = "/nonexistent-xyz/file";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3183-{tag}-{}", std::process::id()));
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

fn expect_redirect_line(root: &Path, body: &str, line: usize) {
    let out = run(root, body);
    assert_eq!(out.rc, 1, "body: {body:?}");
    assert_eq!(
        out.stderr,
        format!("case.sh: line {line}: {BAD}: No such file or directory\n"),
        "body: {body:?}"
    );
}

/// C5 (battery form): a `for' inside `( ... )' reports the
/// subshell's parse-end line (the `)' line), not the `for' line.
#[test]
fn for_in_subshell_reports_subshell_end_line() {
    let root = scratch("c5");
    let out = run(
        &root,
        &format!("(set -e\nfor f in 1 2; do x=4; done > {BAD}\necho after)\n"),
    );
    assert_eq!(out.rc, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!("case.sh: line 3: {BAD}: No such file or directory\n")
    );
}

/// A top-level multiline `for' reports its `done' line.
#[test]
fn toplevel_for_reports_done_line() {
    let root = scratch("m1");
    expect_redirect_line(&root, &format!("for f in 1 2\ndo x=4\ndone > {BAD}\n"), 3);
}

/// A `for' inside a brace group reports the group's closing line:
/// the group body runs under the group's parse-end pin.
#[test]
fn for_in_brace_group_reports_group_end_line() {
    let root = scratch("m9");
    expect_redirect_line(
        &root,
        &format!("{{\nfor f in 1 2; do x=4; done > {BAD}\n}}\n"),
        3,
    );
}

/// The fd-0 face of the same rule: an INPUT redirect failing on a
/// `for' inside a subshell reports the subshell's end line (the
/// do_redirections pre-check composes through the same ambient
/// prefix as the output face).
#[test]
fn for_input_redirect_in_subshell_reports_end_line() {
    let root = scratch("q7");
    expect_redirect_line(&root, &format!("(\nfor i in 1; do :; done < {BAD}\n)\n"), 3);
}

/// C6 (battery form): a function DEFINITION's own diagnostic (the
/// POSIX special-builtin name refusal) reports the ambient line —
/// inside `( ... )', the subshell's closing line.
#[test]
fn function_definition_name_error_reports_ambient_line() {
    let root = scratch("c6");
    let out = run(
        &root,
        "( set -o posix\nbreak()\n{\n echo FUNCNAME: $FUNCNAME\n}\necho after\n)\n",
    );
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: line 7: `break': is a special builtin\n"
    );
}

/// The same refusal at top level reports the definition's own
/// parse-end line (the `}' line).
#[test]
fn function_definition_name_error_toplevel() {
    let root = scratch("m8");
    let out = run(
        &root,
        "set -o posix\nbreak()\n{\n echo FUNCNAME: $FUNCNAME\n}\necho after\n",
    );
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: line 5: `break': is a special builtin\n"
    );
}

/// cm_arith_for keeps the `for' keyword's line (the
/// executing_line_number special case), for output and input alike.
#[test]
fn arith_for_reports_keyword_line() {
    let root = scratch("af");
    expect_redirect_line(
        &root,
        &format!("for ((i=0;i<1;i++))\ndo :\ndone > {BAD}\n"),
        1,
    );
    expect_redirect_line(
        &root,
        &format!("for ((i=0;i<1;i++))\ndo :\ndone < {BAD}\n"),
        1,
    );
}

/// cm_arith / cm_cond report their COMPLETION line: a construct
/// closed on a later physical line reports that line.
#[test]
fn arith_and_cond_report_completion_line() {
    let root = scratch("ac-cc");
    expect_redirect_line(&root, &format!("(( x = 1 \\\n)) > {BAD}\n"), 2);
    expect_redirect_line(&root, &format!("[[ a == a &&\nb == b ]] > {BAD}\n"), 2);
    expect_redirect_line(&root, &format!("[[ a == a &&\nb == b ]] < {BAD}\n"), 2);
    // Single-line forms keep line 1.
    expect_redirect_line(&root, &format!("(( x = 1 )) > {BAD}\n"), 1);
    expect_redirect_line(&root, &format!("[[ a == a ]] > {BAD}\n"), 1);
}

/// C7a (battery form): an eval'd definition whose body ends in a
/// line continuation — the EOF error reports the line after the
/// string's last physical line, and the following command still
/// runs (its own redirect failure reports its own line).
#[test]
fn eval_eof_continuation_reports_following_line() {
    let root = scratch("c7a");
    let out = run(&root, "eval 'X() { (a)>\\'\n: < cve7169-bad2\n");
    assert_eq!(out.rc, 1);
    assert_eq!(
        out.stderr,
        "case.sh: eval: line 3: syntax error: unexpected end of file from `{' command on line 1\n\
         case.sh: line 2: cve7169-bad2: No such file or directory\n"
    );
}

/// The eval EOF family: an unclosed `{` in an eval string reports
/// the line after the string's last line (no continuation needed —
/// the bump is the string-exhaustion rule, not a brace-only one).
#[test]
fn eval_unclosed_brace_reports_following_line() {
    let root = scratch("v5");
    let out = run(&root, "eval 'g() {'\n");
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: eval: line 2: syntax error: unexpected end of file from `{' command on line 1\n"
    );
    let out = run(&root, "eval 'g() { \\'\n");
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: eval: line 3: syntax error: unexpected end of file from `{' command on line 1\n"
    );
}

/// A redirect failure inside a function BODY reports the call
/// frame's line state as GNU's do_redirections sees it: a simple
/// command in the body keeps its own line.
#[test]
fn body_simple_command_keeps_own_line() {
    let root = scratch("c8");
    let out = run(&root, &format!("f() {{\nfalse > {BAD}\n}}\nf\n"));
    assert_eq!(out.rc, 1);
    assert_eq!(
        out.stderr,
        format!("case.sh: line 2: {BAD}: No such file or directory\n")
    );
}
