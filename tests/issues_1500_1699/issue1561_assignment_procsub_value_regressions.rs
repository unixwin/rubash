//! Issue rubash#1561: a standalone assignment whose right-hand side
//! is a process substitution kept the LITERAL span text as the
//! variable's value (`v=>(echo BODY >&2)` stored `>(echo BODY >&2)`)
//! whenever the body's token-join RE-PRINT diverged from the verbatim
//! text (`>&2` re-prints as `>& 2`; pipes, `;`, and quoted redirect
//! targets diverge the same way). The body still ran exactly once —
//! only the value was never spliced.
//!
//! GNU model: extract_process_subst (subst.c:1311) pulls the span
//! VERBATIM out of the word string and process_substitute
//! (subst.c:6362) splices its `/dev/fd/N` result into the expanding
//! word; assignment words receive process substitution exactly like
//! argument words (expand_word_internal, subst.c:11349-11378).
//! Rubash's assignment materialize channels spliced by the re-printed
//! metadata target and silently missed; they now fall back to
//! locating the verbatim span, verified against the source re-print.
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files
//! and -c strings). Sibling gates: issue1523 (finish contract) and
//! issue1557 (inherited-fd forms) must stay green.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
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

#[test]
fn issue1561_output_procsub_whole_rhs_value() {
    // The issue case: the body's `>&2` re-prints as `>& 2`, so the
    // re-printed target never matched the verbatim span.
    let (stdout, stderr, code) = rubash("v=>(echo BODY >&2); echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("v=[/dev/fd/") && stdout.ends_with("]\n"),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "BODY\n", "body must run exactly once");
}

#[test]
fn issue1561_input_procsub_pipe_body_value() {
    // A pipe inside the body diverges the same way (`|` re-prints
    // space-joined); the value is the fd path. GNU's substitution
    // fd is closed when the assignment command finishes, so the
    // later external `cat "$v"` fails on the dangling path — and
    // rubash's fd word behaves identically for an external reader
    // (verified byte-for-byte against GNU 5.3.20, rc included).
    let (stdout, stderr, code) = rubash("v=<(echo body|tr a-z A-Z); echo \"v=[$v]\"; cat \"$v\"");
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(stdout.starts_with("v=[/dev/fd/"), "stdout: {stdout}");
    assert!(stdout.ends_with("]\n"), "stdout: {stdout}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1561_semicolon_and_redirect_bodies() {
    // `;`-separated commands and an attached redirect target in the
    // body are both re-print-divergent.
    let (stdout, stderr, code) =
        rubash("a=>(echo ONE; echo TWO >&2); b=>(true 2>/dev/null); echo \"a=[$a] b=[$b]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("a=[/dev/fd/"), "stdout: {stdout}");
    assert!(stdout.contains("b=[/dev/fd/"), "stdout: {stdout}");
    assert_eq!(stderr.matches("TWO").count(), 1, "stderr: {stderr}");
}

#[test]
fn issue1561_quoted_redirect_target_body() {
    // `>&"2"` — the quoted redirect target keeps its quotes verbatim
    // in the word but not in the re-print.
    let (stdout, stderr, code) = rubash("v=>(echo BODY >&\"2\"); echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("v=[/dev/fd/") && stdout.ends_with("]\n"),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "BODY\n");
}

#[test]
fn issue1561_two_spans_first_divergent() {
    // First span word-initial (re-print target), second embedded
    // (verbatim target): both must splice, in order.
    let (stdout, stderr, code) = rubash("v=>(echo A >&2)mid>(echo B >&2); echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("v=[/dev/fd/") && stdout.contains("mid/dev/fd/"),
        "stdout: {stdout}"
    );
    assert_eq!(stderr.matches('A').count(), 1, "stderr: {stderr}");
    assert_eq!(stderr.matches('B').count(), 1, "stderr: {stderr}");
}

#[test]
fn issue1561_suffix_after_span() {
    // A suffix after the span (`>(...)suf`) keeps the suffix after
    // the spliced fd path.
    let (stdout, stderr, code) = rubash("v=>(echo A >&2)suf; echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("v=[/dev/fd/") && stdout.ends_with("suf]\n"),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "A\n");
}

#[test]
fn issue1561_body_runs_exactly_once() {
    // The #1523 exactly-once contract on the newly spliced forms:
    // the body runs once at the assignment's own finish, and the
    // assignment's status stays clean.
    let (stdout, stderr, code) = rubash("v=>(echo MARK >&2)\necho \"rc=$? v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr.matches("MARK").count(), 1, "stderr: {stderr}");
    assert!(stdout.contains("rc=0 v=[/dev/fd/"), "stdout: {stdout}");
}

#[test]
fn issue1561_quoted_span_stays_literal() {
    // A quoted span is data, not a substitution (subst.c:11349-11378
    // only extracts UNQUOTED spans): no splice, no body run.
    let (stdout, stderr, code) = rubash("v='>(echo BODY >&2)'; echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v=[>(echo BODY >&2)]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1561_declare_form_value_control() {
    // Control: the declare form already materialized the value; it
    // must keep doing so.
    let (stdout, stderr, code) = rubash("declare v=>(echo BODY >&2); echo \"v=[$v]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("v=[/dev/fd/") && stdout.ends_with("]\n"),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "BODY\n");
}
